//! A writer for the Java Object Serialization Stream Protocol: the bytes
//! `java.io.ObjectOutputStream` writes, for the classes [`ClassSpec`] tables
//! describe (see [`crate::wp_classes`]), without a JVM.
//!
//! An object is written from the values of its fields, by name; the writer
//! puts them in the order of its class descriptors, super class first, with
//! zeros and nulls for the fields not given, and the data a class's own
//! `writeObject` adds (a `HashMap`'s entries, an `ArrayList`'s items) where
//! the protocol wants it. Class descriptors, enum constants and the objects
//! a caller asks to share are written once, then referred to by handle.
//!
//! [`GzipWriter`] compresses the stream on every core, as one gzip member
//! that any inflater reads.

use flate2::{Compress, Compression, Crc, FlushCompress, Status};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::sync::OnceLock;
use std::sync::mpsc;

const MAGIC: u16 = 0xACED;
const VERSION: u16 = 5;
const BASE_HANDLE: i32 = 0x7E_0000;

const TC_NULL: u8 = 0x70;
const TC_REFERENCE: u8 = 0x71;
const TC_CLASSDESC: u8 = 0x72;
const TC_OBJECT: u8 = 0x73;
const TC_STRING: u8 = 0x74;
const TC_ARRAY: u8 = 0x75;
const TC_BLOCKDATA: u8 = 0x77;
const TC_ENDBLOCKDATA: u8 = 0x78;
const TC_BLOCKDATALONG: u8 = 0x7A;
const TC_LONGSTRING: u8 = 0x7C;
const TC_ENUM: u8 = 0x7E;

const SC_WRITE_METHOD: u8 = 0x01;

/// A class as its serialisation descriptor tells it: name, serial version
/// UID, flags, the serialisable super class and the fields in stream order
/// (type code, name, type string of an object field). Enums list their
/// constants.
#[derive(Debug)]
pub struct ClassSpec {
    pub name: &'static str,
    pub suid: i64,
    pub flags: u8,
    pub sup: Option<&'static str>,
    pub fields: &'static [(u8, &'static str, &'static str)],
    pub constants: &'static [&'static str],
}

fn specs() -> &'static HashMap<&'static str, &'static ClassSpec> {
    static SPECS: OnceLock<HashMap<&'static str, &'static ClassSpec>> = OnceLock::new();
    SPECS.get_or_init(|| crate::wp_classes::CLASSES.iter().map(|c| (c.name, c)).collect())
}

/// The descriptor of class `name`.
pub fn spec(name: &str) -> io::Result<&'static ClassSpec> {
    specs().get(name).copied().ok_or_else(|| invalid(format!("no descriptor for class {name}")))
}

fn invalid(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what)
}

/// Something that writes one object (or a null, or a reference) of a
/// field, an entry or an item.
pub type Put<'a> = Box<dyn FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a>;

/// The value of a field.
pub enum Value<'a> {
    Bool(bool),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    /// An object field: writes the object.
    Put(Put<'a>),
}

/// The values of an object's fields, by name, and what its classes'
/// `writeObject` methods add.
pub struct Obj<'a> {
    /// (declaring class, field, value): the class is that of the object
    /// unless given.
    values: Vec<(Option<&'static str>, &'static str, Option<Value<'a>>)>,
    custom: Vec<(&'static str, Option<Put<'a>>)>,
}

impl<'a> Obj<'a> {
    fn new() -> Self {
        Self { values: Vec::new(), custom: Vec::new() }
    }

    /// Sets a field of the object's own class, or of the nearest super
    /// class that has a field of that name.
    pub fn set(&mut self, field: &'static str, value: Value<'a>) -> &mut Self {
        self.values.push((None, field, Some(value)));
        self
    }

    /// Sets a field of the given class of the object's (for a name two of
    /// its classes have).
    pub fn set_in(&mut self, class: &'static str, field: &'static str, value: Value<'a>) -> &mut Self {
        self.values.push((Some(class), field, Some(value)));
        self
    }

    pub fn bool(&mut self, field: &'static str, v: bool) -> &mut Self {
        self.set(field, Value::Bool(v))
    }
    pub fn byte(&mut self, field: &'static str, v: i8) -> &mut Self {
        self.set(field, Value::Byte(v))
    }
    pub fn short(&mut self, field: &'static str, v: i16) -> &mut Self {
        self.set(field, Value::Short(v))
    }
    pub fn int(&mut self, field: &'static str, v: i32) -> &mut Self {
        self.set(field, Value::Int(v))
    }
    pub fn long(&mut self, field: &'static str, v: i64) -> &mut Self {
        self.set(field, Value::Long(v))
    }
    pub fn float(&mut self, field: &'static str, v: f32) -> &mut Self {
        self.set(field, Value::Float(v))
    }
    pub fn double(&mut self, field: &'static str, v: f64) -> &mut Self {
        self.set(field, Value::Double(v))
    }
    /// An object field, written by `put`.
    pub fn put(&mut self, field: &'static str, put: impl FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a) -> &mut Self {
        self.set(field, Value::Put(Box::new(put)))
    }
    /// A string field.
    pub fn string(&mut self, field: &'static str, s: impl Into<String>) -> &mut Self {
        let s = s.into();
        self.put(field, move |w| w.string(&s).map(drop))
    }
    /// An enum field.
    pub fn enumeration(&mut self, field: &'static str, class: &'static str, constant: &'static str) -> &mut Self {
        self.put(field, move |w| w.enum_const(class, constant).map(drop))
    }
    /// A reference to an object written before, by its handle.
    pub fn reference(&mut self, field: &'static str, handle: i32) -> &mut Self {
        self.put(field, move |w| w.reference(handle))
    }
    /// What `class`'s `writeObject` writes after its fields: primitives
    /// through the `block_` methods, objects as anywhere.
    pub fn custom(&mut self, class: &'static str, put: impl FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a) -> &mut Self {
        self.custom.push((class, Some(Box::new(put))));
        self
    }
}

/// Writes a Java serialisation stream to a sink.
pub struct Writer<'s> {
    sink: &'s mut dyn Write,
    buf: Vec<u8>,
    /// The handle the next new item gets, from 0.
    next: i32,
    descs: HashMap<&'static str, i32>,
    types: HashMap<&'static str, i32>,
    enums: HashMap<(&'static str, &'static str), i32>,
    shared: HashMap<u64, i32>,
    /// Primitives a `writeObject` wrote, not yet framed as block data.
    block: Vec<u8>,
    /// The objects whose fields are being written, innermost last.
    stack: Vec<i32>,
}

const FLUSH_AT: usize = 4 << 20;

impl<'s> Writer<'s> {
    /// Starts a stream: its magic number and version.
    pub fn new(sink: &'s mut dyn Write) -> Self {
        let mut w = Self {
            sink,
            buf: Vec::with_capacity(FLUSH_AT + (1 << 20)),
            next: 0,
            descs: HashMap::new(),
            types: HashMap::new(),
            enums: HashMap::new(),
            shared: HashMap::new(),
            block: Vec::new(),
            stack: Vec::new(),
        };
        w.u16(MAGIC);
        w.u16(VERSION);
        w
    }

    /// Writes out what is buffered.
    pub fn finish(mut self) -> io::Result<()> {
        self.flush_block();
        self.sink.write_all(&self.buf)?;
        self.buf.clear();
        self.sink.flush()
    }

    fn spill(&mut self) -> io::Result<()> {
        if self.buf.len() >= FLUSH_AT {
            self.sink.write_all(&self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn utf(&mut self, s: &str) {
        let bytes = modified_utf8(s);
        self.u16(bytes.len() as u16);
        self.buf.extend_from_slice(&bytes);
    }

    fn new_handle(&mut self) -> i32 {
        self.next += 1;
        BASE_HANDLE + self.next - 1
    }

    /// The handle of the object whose fields are being written.
    pub fn current(&self) -> i32 {
        *self.stack.last().expect("inside an object")
    }

    /// The handle of the object shared under `key`, once written.
    pub fn shared(&self, key: u64) -> Option<i32> {
        self.shared.get(&key).copied()
    }

    /// Remembers an object written to be shared under `key`.
    pub fn remember(&mut self, key: u64, handle: i32) {
        self.shared.insert(key, handle);
    }

    /// Block data, inside a `writeObject`'s part.
    pub fn block_bytes(&mut self, b: &[u8]) {
        self.block.extend_from_slice(b);
    }
    pub fn block_i32(&mut self, v: i32) {
        self.block.extend_from_slice(&v.to_be_bytes());
    }
    pub fn block_f32(&mut self, v: f32) {
        self.block.extend_from_slice(&v.to_bits().to_be_bytes());
    }
    pub fn block_u16(&mut self, v: u16) {
        self.block.extend_from_slice(&v.to_be_bytes());
    }

    /// Frames the pending block data as `ObjectOutputStream` does: chunks
    /// of at most 1024 bytes.
    fn flush_block(&mut self) {
        if self.block.is_empty() {
            return;
        }
        let block = std::mem::take(&mut self.block);
        for chunk in block.chunks(1024) {
            if chunk.len() <= 255 {
                self.u8(TC_BLOCKDATA);
                self.u8(chunk.len() as u8);
            } else {
                self.u8(TC_BLOCKDATALONG);
                self.i32(chunk.len() as i32);
            }
            self.buf.extend_from_slice(chunk);
        }
    }

    pub fn null(&mut self) -> io::Result<()> {
        self.flush_block();
        self.u8(TC_NULL);
        Ok(())
    }

    pub fn reference(&mut self, handle: i32) -> io::Result<()> {
        self.flush_block();
        self.u8(TC_REFERENCE);
        self.i32(handle);
        Ok(())
    }

    /// A new string object.
    pub fn string(&mut self, s: &str) -> io::Result<i32> {
        self.flush_block();
        let bytes = modified_utf8(s);
        if bytes.len() <= u16::MAX as usize {
            self.u8(TC_STRING);
            let h = self.new_handle();
            self.u16(bytes.len() as u16);
            self.buf.extend_from_slice(&bytes);
            Ok(h)
        } else {
            self.u8(TC_LONGSTRING);
            let h = self.new_handle();
            self.i64(bytes.len() as i64);
            self.buf.extend_from_slice(&bytes);
            Ok(h)
        }
    }

    /// An enum constant, by its class and name.
    pub fn enum_const(&mut self, class: &'static str, name: &'static str) -> io::Result<i32> {
        if let Some(&h) = self.enums.get(&(class, name)) {
            self.reference(h)?;
            return Ok(h);
        }
        let s = spec(class)?;
        if !s.constants.contains(&name) {
            return Err(invalid(format!("{class} has no constant {name}")));
        }
        self.flush_block();
        self.u8(TC_ENUM);
        self.class_desc(class)?;
        let h = self.new_handle();
        self.enums.insert((class, name), h);
        self.string(name)?;
        Ok(h)
    }

    fn class_desc(&mut self, name: &str) -> io::Result<()> {
        if let Some(&h) = self.descs.get(name) {
            self.u8(TC_REFERENCE);
            self.i32(h);
            return Ok(());
        }
        let s = spec(name)?;
        self.u8(TC_CLASSDESC);
        let h = self.new_handle();
        self.descs.insert(s.name, h);
        self.utf(s.name);
        self.i64(s.suid);
        self.u8(s.flags);
        self.u16(s.fields.len() as u16);
        for &(code, field, signature) in s.fields {
            self.u8(code);
            self.utf(field);
            if code == b'L' || code == b'[' {
                match self.types.get(signature) {
                    Some(&t) => {
                        self.u8(TC_REFERENCE);
                        self.i32(t);
                    }
                    None => {
                        let t = self.string(signature)?;
                        self.types.insert(signature, t);
                    }
                }
            }
        }
        // No class annotation.
        self.u8(TC_ENDBLOCKDATA);
        match s.sup {
            Some(sup) => self.class_desc(sup),
            None => {
                self.u8(TC_NULL);
                Ok(())
            }
        }
    }

    /// An object of `class`, its fields as `fill` sets them.
    pub fn object<'a>(&mut self, class: &'static str, fill: impl FnOnce(&mut Obj<'a>)) -> io::Result<i32> {
        let mut obj = Obj::new();
        fill(&mut obj);
        let mut chain = Vec::new();
        let mut at = Some(class);
        while let Some(name) = at {
            let s = spec(name)?;
            chain.push(s);
            at = s.sup;
        }
        // Each value to the most derived class that has its field.
        for (owner, field, _) in &mut obj.values {
            if owner.is_none() {
                let declaring = chain.iter().find(|s| s.fields.iter().any(|f| f.1 == *field));
                *owner = Some(declaring.ok_or_else(|| invalid(format!("{class} has no field {field}")))?.name);
            }
        }
        chain.reverse();
        self.flush_block();
        self.u8(TC_OBJECT);
        self.class_desc(class)?;
        let h = self.new_handle();
        self.stack.push(h);
        for s in &chain {
            for &(code, field, _) in s.fields {
                let value = obj.values.iter_mut().find(|v| v.0 == Some(s.name) && v.1 == field && v.2.is_some()).and_then(|v| v.2.take());
                self.field(class, field, code, value)?;
            }
            if s.flags & SC_WRITE_METHOD != 0 {
                if let Some(put) = obj.custom.iter_mut().find(|c| c.0 == s.name).and_then(|c| c.1.take()) {
                    put(self)?;
                }
                self.flush_block();
                self.u8(TC_ENDBLOCKDATA);
            }
            self.spill()?;
        }
        self.stack.pop();
        if let Some((owner, field, _)) = obj.values.iter().find(|v| v.2.is_some()) {
            return Err(invalid(format!("{class}: field {field} of {} not written", owner.unwrap_or("?"))));
        }
        if let Some((owner, _)) = obj.custom.iter().find(|c| c.1.is_some()) {
            return Err(invalid(format!("{class}: {owner} writes no data of its own")));
        }
        Ok(h)
    }

    fn field(&mut self, class: &str, field: &str, code: u8, value: Option<Value<'_>>) -> io::Result<()> {
        match (code, value) {
            (b'Z', Some(Value::Bool(v))) => self.u8(v as u8),
            (b'B', Some(Value::Byte(v))) => self.u8(v as u8),
            (b'S', Some(Value::Short(v))) => self.u16(v as u16),
            (b'I', Some(Value::Int(v))) => self.i32(v),
            (b'J', Some(Value::Long(v))) => self.i64(v),
            (b'F', Some(Value::Float(v))) => self.i32(v.to_bits() as i32),
            (b'D', Some(Value::Double(v))) => self.i64(v.to_bits() as i64),
            (b'L' | b'[', Some(Value::Put(put))) => put(self)?,
            (b'Z' | b'B', None) => self.u8(0),
            (b'S' | b'C', None) => self.u16(0),
            (b'I' | b'F', None) => self.i32(0),
            (b'J' | b'D', None) => self.i64(0),
            (b'L' | b'[', None) => self.u8(TC_NULL),
            _ => return Err(invalid(format!("{class}: wrong type of value for field {field}"))),
        }
        Ok(())
    }

    fn array_head(&mut self, class: &'static str, len: usize) -> io::Result<i32> {
        self.flush_block();
        self.u8(TC_ARRAY);
        self.class_desc(class)?;
        let h = self.new_handle();
        self.i32(len as i32);
        Ok(h)
    }

    pub fn byte_array(&mut self, v: &[u8]) -> io::Result<i32> {
        let h = self.array_head("[B", v.len())?;
        self.buf.extend_from_slice(v);
        self.spill()?;
        Ok(h)
    }

    pub fn short_array(&mut self, v: &[u16]) -> io::Result<i32> {
        let h = self.array_head("[S", v.len())?;
        self.buf.reserve(v.len() * 2);
        for x in v {
            self.buf.extend_from_slice(&x.to_be_bytes());
        }
        self.spill()?;
        Ok(h)
    }

    pub fn int_array(&mut self, v: &[i32]) -> io::Result<i32> {
        let h = self.array_head("[I", v.len())?;
        self.buf.reserve(v.len() * 4);
        for x in v {
            self.buf.extend_from_slice(&x.to_be_bytes());
        }
        self.spill()?;
        Ok(h)
    }

    pub fn long_array(&mut self, v: &[i64]) -> io::Result<i32> {
        let h = self.array_head("[J", v.len())?;
        self.buf.reserve(v.len() * 8);
        for x in v {
            self.buf.extend_from_slice(&x.to_be_bytes());
        }
        self.spill()?;
        Ok(h)
    }

    /// An array of objects (of the array class `class`, `[L…;`), its items
    /// written by `items`.
    pub fn object_array<'a>(&mut self, class: &'static str, items: Vec<Put<'a>>) -> io::Result<i32> {
        let h = self.array_head(class, items.len())?;
        for put in items {
            put(self)?;
        }
        Ok(h)
    }

    /// A `java.lang.Integer`.
    pub fn integer(&mut self, v: i32) -> io::Result<i32> {
        self.object("java.lang.Integer", |o| {
            o.int("value", v);
        })
    }

    /// A `java.lang.Boolean`.
    pub fn boolean(&mut self, v: bool) -> io::Result<i32> {
        self.object("java.lang.Boolean", |o| {
            o.bool("value", v);
        })
    }

    /// A `java.awt.Point`.
    pub fn point(&mut self, x: i32, y: i32) -> io::Result<i32> {
        self.object("java.awt.Point", |o| {
            o.int("x", x).int("y", y);
        })
    }

    /// A `java.util.UUID`.
    pub fn uuid(&mut self, uuid: u128) -> io::Result<i32> {
        self.object("java.util.UUID", |o| {
            o.long("mostSigBits", (uuid >> 64) as i64).long("leastSigBits", uuid as i64);
        })
    }

    /// A `java.util.ArrayList` of the items `items` writes, `len` of them.
    pub fn array_list<'a>(&mut self, len: usize, items: impl FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a) -> io::Result<i32> {
        self.object("java.util.ArrayList", |o| {
            o.int("size", len as i32).custom("java.util.ArrayList", move |w| {
                w.block_i32(len as i32);
                items(w)
            });
        })
    }

    /// A `java.util.HashMap` of `len` entries, which `entries` writes:
    /// key, value, key, value…
    pub fn hash_map<'a>(&mut self, len: usize, entries: impl FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a) -> io::Result<i32> {
        let capacity = table_size(len);
        self.object("java.util.HashMap", |o| {
            o.float("loadFactor", 0.75).int("threshold", (capacity as f32 * 0.75) as i32).custom("java.util.HashMap", move |w| {
                w.block_i32(capacity as i32);
                w.block_i32(len as i32);
                entries(w)
            });
        })
    }

    /// A `java.util.HashSet` of `len` items, which `items` writes.
    pub fn hash_set<'a>(&mut self, len: usize, items: impl FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a) -> io::Result<i32> {
        let capacity = table_size(len);
        self.object("java.util.HashSet", |o| {
            o.custom("java.util.HashSet", move |w| {
                w.block_i32(capacity as i32);
                w.block_f32(0.75);
                w.block_i32(len as i32);
                items(w)
            });
        })
    }

    /// A `java.util.TreeMap` (natural order) of `len` entries, which
    /// `entries` writes in their order: key, value, key, value…
    pub fn tree_map<'a>(&mut self, len: usize, entries: impl FnOnce(&mut Writer<'_>) -> io::Result<()> + 'a) -> io::Result<i32> {
        self.object("java.util.TreeMap", |o| {
            o.custom("java.util.TreeMap", move |w| {
                w.block_i32(len as i32);
                entries(w)
            });
        })
    }

    /// A `java.util.BitSet` of these bits.
    pub fn bit_set(&mut self, bits: &[bool]) -> io::Result<i32> {
        let mut words = vec![0i64; bits.len().div_ceil(64)];
        for (i, _) in bits.iter().enumerate().filter(|b| *b.1) {
            words[i / 64] |= 1 << (i % 64);
        }
        // As BitSet keeps them: up to the last word with a bit set.
        while words.last() == Some(&0) {
            words.pop();
        }
        self.object("java.util.BitSet", |o| {
            o.put("bits", move |w| w.long_array(&words).map(drop));
        })
    }
}

/// The table size a `HashMap` of `len` entries has.
fn table_size(len: usize) -> usize {
    let needed = ((len as f64 / 0.75).ceil() as usize).max(1);
    needed.next_power_of_two().max(16)
}

/// Java's modified UTF-8: NUL as two bytes, characters past the BMP as
/// surrogate pairs of three bytes each.
pub fn modified_utf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            1..=0x7F => out.push(unit as u8),
            0 | 0x80..=0x7FF => {
                out.push(0xC0 | (unit >> 6) as u8);
                out.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                out.push(0xE0 | (unit >> 12) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                out.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    out
}

/// A gzip stream compressed on several threads: the input is cut into
/// blocks, each deflated on its own and ended on a byte boundary (a sync
/// flush), the last one finished; together they are one deflate stream, in
/// one gzip member, as `java.util.zip.GZIPInputStream` reads it.
pub struct GzipWriter<W: Write> {
    out: Option<W>,
    block: Vec<u8>,
    level: u32,
    jobs: Option<mpsc::Sender<Job>>,
    workers: Vec<std::thread::JoinHandle<()>>,
    pending: VecDeque<mpsc::Receiver<io::Result<(Vec<u8>, Crc)>>>,
    crc: Crc,
    /// Bytes written by the caller.
    pub total_in: u64,
    /// Compressed bytes written out.
    pub total_out: u64,
}

struct Job {
    input: Vec<u8>,
    last: bool,
    level: u32,
    done: mpsc::SyncSender<io::Result<(Vec<u8>, Crc)>>,
}

const GZIP_BLOCK: usize = 2 << 20;

impl<W: Write> GzipWriter<W> {
    /// A gzip stream at `level` (0 to 9) on `out`, whose header carries
    /// `extra` (subfields of the gzip extra field) if given.
    pub fn new(mut out: W, level: u32, extra: Option<&[u8]>) -> io::Result<Self> {
        let flags = if extra.is_some() { 0x04 } else { 0 };
        out.write_all(&[0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 0xFF])?;
        let mut total_out = 10;
        if let Some(extra) = extra {
            out.write_all(&(extra.len() as u16).to_le_bytes())?;
            out.write_all(extra)?;
            total_out += 2 + extra.len() as u64;
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 12);
        let (send, receive) = mpsc::channel::<Job>();
        let receive = std::sync::Arc::new(std::sync::Mutex::new(receive));
        let workers = (0..threads)
            .map(|k| {
                let receive = receive.clone();
                std::thread::Builder::new()
                    .name(format!("massif-gzip-{k}"))
                    .spawn(move || {
                        loop {
                            let job = match receive.lock().map(|r| r.recv()) {
                                Ok(Ok(job)) => job,
                                _ => return,
                            };
                            let _ = job.done.send(deflate(&job.input, job.level, job.last));
                        }
                    })
                    .expect("a thread starts")
            })
            .collect();
        Ok(Self { out: Some(out), block: Vec::with_capacity(GZIP_BLOCK), level, jobs: Some(send), workers, pending: VecDeque::new(), crc: Crc::new(), total_in: 0, total_out })
    }

    fn dispatch(&mut self, last: bool) -> io::Result<()> {
        let input = std::mem::replace(&mut self.block, Vec::with_capacity(if last { 0 } else { GZIP_BLOCK }));
        let (done, result) = mpsc::sync_channel(1);
        self.jobs.as_ref().expect("not finished").send(Job { input, last, level: self.level, done }).map_err(|_| io::Error::other("the compressor stopped"))?;
        self.pending.push_back(result);
        let most = self.workers.len() * 2;
        while self.pending.len() > most || (last && !self.pending.is_empty()) {
            self.drain_one()?;
        }
        Ok(())
    }

    fn drain_one(&mut self) -> io::Result<()> {
        let result = self.pending.pop_front().expect("pending");
        let (bytes, crc) = result.recv().map_err(|_| io::Error::other("the compressor stopped"))??;
        self.out.as_mut().expect("not finished").write_all(&bytes)?;
        self.total_out += bytes.len() as u64;
        self.crc.combine(&crc);
        Ok(())
    }

    /// Ends the stream: the last block, the CRC and the length.
    pub fn finish(mut self) -> io::Result<W> {
        self.dispatch(true)?;
        let (crc, size) = (self.crc.sum(), self.crc.amount());
        let mut out = self.out.take().expect("not finished");
        out.write_all(&crc.to_le_bytes())?;
        out.write_all(&size.to_le_bytes())?;
        self.total_out += 8;
        out.flush()?;
        Ok(out)
    }
}

impl<W: Write> Drop for GzipWriter<W> {
    fn drop(&mut self) {
        drop(self.jobs.take());
        for w in std::mem::take(&mut self.workers) {
            let _ = w.join();
        }
    }
}

impl<W: Write> Write for GzipWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut rest = data;
        while !rest.is_empty() {
            let n = rest.len().min(GZIP_BLOCK - self.block.len());
            self.block.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            if self.block.len() == GZIP_BLOCK {
                self.dispatch(false)?;
            }
        }
        self.total_in += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Raw deflate of `input`, ended on a byte boundary, or finished.
fn deflate(input: &[u8], level: u32, last: bool) -> io::Result<(Vec<u8>, Crc)> {
    let mut crc = Crc::new();
    crc.update(input);
    let mut z = Compress::new(Compression::new(level), false);
    let mut out = Vec::with_capacity(input.len() / 2 + 1024);
    let flush = if last { FlushCompress::Finish } else { FlushCompress::Sync };
    loop {
        let consumed = z.total_in() as usize;
        if out.capacity() - out.len() < 4096 {
            out.reserve(out.capacity().max(64 << 10));
        }
        let status = z.compress_vec(&input[consumed..], &mut out, flush).map_err(io::Error::other)?;
        let done_input = z.total_in() as usize == input.len();
        match status {
            Status::StreamEnd => break,
            // A sync flush is done once all the input is in and there was
            // room left over.
            Status::Ok | Status::BufError if !last && done_input && out.capacity() - out.len() >= 64 => break,
            _ => {}
        }
    }
    Ok((out, crc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::javaser::{Array, Parser, Value as Read};

    #[test]
    fn modified_utf8_is_javas() {
        assert_eq!(modified_utf8("abc"), b"abc");
        assert_eq!(modified_utf8("\0"), [0xC0, 0x80]);
        assert_eq!(modified_utf8("é"), "é".as_bytes());
        assert_eq!(modified_utf8("😀"), [0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
        assert_eq!(crate::javaser::decode_modified_utf8(&modified_utf8("a\0b😀é")), "a\0b😀é");
    }

    #[test]
    fn what_is_written_reads_back() {
        let mut bytes = Vec::new();
        {
            let mut w = Writer::new(&mut bytes);
            w.hash_map(2, |w| {
                w.point(3, -4)?;
                w.object("org.pepsoft.worldpainter.Tile", |o| {
                    o.int("x", 3).int("y", -4).int("minHeight", -64).int("maxHeight", 320).bool("tall", true).put("tallHeightMap", |w| w.int_array(&[256, 512]).map(drop)).put("terrain", |w| w.byte_array(&[1, 2]).map(drop));
                })?;
                w.string("key")?;
                w.array_list(2, |w| {
                    w.enum_const("org.pepsoft.worldpainter.Terrain", "GRASS")?;
                    w.enum_const("org.pepsoft.worldpainter.Terrain", "GRASS").map(drop)
                })
                .map(drop)
            })
            .unwrap();
            w.bit_set(&[true, false, true]).unwrap();
            w.finish().unwrap();
        }
        let mut p = Parser::new(&bytes[..]).unwrap();
        let map = p.next_object().unwrap().unwrap();
        let bits = p.next_object().unwrap().unwrap();
        assert!(p.next_object().unwrap().is_none());
        let a = &p.arena;
        let entries = a.map_entries(map).unwrap();
        assert_eq!(entries.len(), 2);
        let (key, tile) = entries[0];
        assert_eq!(a.int(a.field(key, "x").unwrap()), Some(3));
        assert_eq!(a.int(a.field(key, "y").unwrap()), Some(-4));
        assert_eq!(a.class_name(tile), Some("org.pepsoft.worldpainter.Tile"));
        assert_eq!(a.int(a.field(tile, "minHeight").unwrap()), Some(-64));
        assert_eq!(a.bool(a.field(tile, "tall").unwrap()), Some(true));
        assert!(matches!(a.array(a.field(tile, "tallHeightMap").unwrap()), Some(Array::Int(v)) if v == &[256, 512]));
        assert!(matches!(a.field(tile, "heightMap"), Some(Read::Null)));
        let items = a.list_items(entries[1].1).unwrap();
        assert_eq!(items.iter().map(|&i| a.enum_name(i)).collect::<Vec<_>>(), [Some("GRASS"), Some("GRASS")]);
        assert!(matches!(a.array(a.field(bits, "bits").unwrap()), Some(Array::Long(v)) if v == &[5]));
    }

    #[test]
    fn unknown_fields_and_types_are_refused() {
        let mut bytes = Vec::new();
        let mut w = Writer::new(&mut bytes);
        assert!(w.object("java.awt.Point", |o| {
            o.int("z", 1);
        }).is_err());
        assert!(w.object("java.awt.Point", |o| {
            o.long("x", 1);
        }).is_err());
        assert!(w.enum_const("org.pepsoft.worldpainter.Terrain", "NOT_A_TERRAIN").is_err());
    }

    #[test]
    fn parallel_gzip_inflates_to_the_input() {
        use std::io::Read as _;
        let input: Vec<u8> = (0..(GZIP_BLOCK * 3 + 12345)).map(|i| ((i * 7919) % 251) as u8 ^ (i >> 13) as u8).collect();
        let mut gz = GzipWriter::new(Vec::new(), 6, Some(b"WP\x02\x00hi")).unwrap();
        for chunk in input.chunks(100_000) {
            gz.write_all(chunk).unwrap();
        }
        let out = gz.finish().unwrap();
        let mut back = Vec::new();
        flate2::read::GzDecoder::new(&out[..]).read_to_end(&mut back).unwrap();
        assert_eq!(back, input);
        assert_eq!(crate::worldpainter::read_thumbnail(&out).as_deref(), Some(&b"hi"[..]));
        // An empty stream too.
        let out = GzipWriter::new(Vec::new(), 6, None).unwrap().finish().unwrap();
        let mut back = Vec::new();
        flate2::read::GzDecoder::new(&out[..]).read_to_end(&mut back).unwrap();
        assert!(back.is_empty());
    }
}
