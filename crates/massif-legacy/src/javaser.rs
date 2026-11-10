//! A reader for the Java Object Serialization Stream Protocol (the format of
//! `java.io.ObjectOutputStream`), written from the specification: no JVM and
//! no knowledge of the classes is needed.
//!
//! Everything the stream contains ends up in an [`Arena`]: class
//! descriptors, objects (field values and the extra data their `writeObject`
//! methods wrote), arrays, strings and enum constants. [`Value::Ref`] points
//! into the arena. A hook can be told when an object is complete, to convert
//! it and free its bulk data while the rest of the stream is still being
//! read, which keeps memory flat on very large streams.

use std::io::{self, Read};

const MAGIC: u16 = 0xACED;
const VERSION: u16 = 5;
const BASE_HANDLE: i32 = 0x7E_0000;

pub(crate) const TC_NULL: u8 = 0x70;
pub(crate) const TC_REFERENCE: u8 = 0x71;
pub(crate) const TC_CLASSDESC: u8 = 0x72;
pub(crate) const TC_OBJECT: u8 = 0x73;
pub(crate) const TC_STRING: u8 = 0x74;
pub(crate) const TC_ARRAY: u8 = 0x75;
const TC_CLASS: u8 = 0x76;
pub(crate) const TC_BLOCKDATA: u8 = 0x77;
pub(crate) const TC_ENDBLOCKDATA: u8 = 0x78;
const TC_RESET: u8 = 0x79;
const TC_BLOCKDATALONG: u8 = 0x7A;
const TC_EXCEPTION: u8 = 0x7B;
const TC_LONGSTRING: u8 = 0x7C;
const TC_PROXYCLASSDESC: u8 = 0x7D;
pub(crate) const TC_ENUM: u8 = 0x7E;

pub(crate) const SC_WRITE_METHOD: u8 = 0x01;
pub(crate) const SC_SERIALIZABLE: u8 = 0x02;
const SC_EXTERNALIZABLE: u8 = 0x04;
const SC_BLOCK_DATA: u8 = 0x08;

/// Nesting deeper than this is treated as a corrupt stream rather than
/// risking the stack.
const MAX_DEPTH: usize = 2500;

#[derive(Debug, thiserror::Error)]
pub enum JavaError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("not a Java serialization stream")]
    NotJava,
    #[error("corrupt Java stream: {0}")]
    Corrupt(String),
    #[error("the stream holds a serialized exception: the file was not written completely")]
    Exception,
    #[error("class {0} uses an old externalizable format that cannot be read without the class")]
    Externalizable(String),
}

pub type Result<T> = std::result::Result<T, JavaError>;

fn corrupt<T>(what: impl Into<String>) -> Result<T> {
    Err(JavaError::Corrupt(what.into()))
}

/// Index into the [`Arena`].
pub type Handle = usize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Byte(i8),
    Char(u16),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Ref(Handle),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub type_code: u8,
    /// Declared type of object fields, in JVM notation (`Ljava/lang/String;`).
    pub class_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClassDesc {
    pub name: String,
    pub suid: i64,
    pub flags: u8,
    pub fields: Vec<Field>,
    pub super_class: Option<Handle>,
}

/// Extra data written by a `writeObject` method: raw bytes and objects.
#[derive(Debug, Clone)]
pub enum Item {
    Block(Vec<u8>),
    Value(Value),
}

/// The part of an object belonging to one class of its hierarchy.
#[derive(Debug, Clone)]
pub struct ClassData {
    pub desc: Handle,
    pub values: Vec<Value>,
    pub annotation: Vec<Item>,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub desc: Handle,
    /// From the topmost serializable superclass down to the object's class.
    pub data: Vec<ClassData>,
}

#[derive(Debug, Clone)]
pub enum Array {
    Bool(Vec<bool>),
    Byte(Vec<u8>),
    Char(Vec<u16>),
    Short(Vec<i16>),
    Int(Vec<i32>),
    Long(Vec<i64>),
    Float(Vec<f32>),
    Double(Vec<f64>),
    Object(Vec<Value>),
}

impl Array {
    pub fn len(&self) -> usize {
        match self {
            Array::Bool(v) => v.len(),
            Array::Byte(v) => v.len(),
            Array::Char(v) => v.len(),
            Array::Short(v) => v.len(),
            Array::Int(v) => v.len(),
            Array::Long(v) => v.len(),
            Array::Float(v) => v.len(),
            Array::Double(v) => v.len(),
            Array::Object(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone)]
pub enum Content {
    /// Being read (an object referring to itself sees this).
    Pending,
    /// Released by a hook after conversion.
    Freed,
    ClassDesc(ClassDesc),
    Object(Object),
    Array { desc: Handle, data: Array },
    String(String),
    Enum { desc: Handle, name: String },
    Class(Handle),
}

/// Everything read from a stream.
#[derive(Debug, Default)]
pub struct Arena {
    pub items: Vec<Content>,
}

impl Arena {
    pub fn get(&self, h: Handle) -> Option<&Content> {
        self.items.get(h)
    }

    pub fn take(&mut self, h: Handle) -> Content {
        match self.items.get_mut(h) {
            Some(c) => std::mem::replace(c, Content::Freed),
            None => Content::Freed,
        }
    }

    fn desc(&self, h: Handle) -> Option<&ClassDesc> {
        match self.items.get(h)? {
            Content::ClassDesc(d) => Some(d),
            _ => None,
        }
    }

    /// The class name of an object, array or enum constant.
    pub fn class_name(&self, v: Value) -> Option<&str> {
        let Value::Ref(h) = v else { return None };
        let desc = match self.items.get(h)? {
            Content::Object(o) => o.desc,
            Content::Array { desc, .. } => *desc,
            Content::Enum { desc, .. } => *desc,
            _ => return None,
        };
        self.desc(desc).map(|d| d.name.as_str())
    }

    pub fn object(&self, v: Value) -> Option<&Object> {
        match v {
            Value::Ref(h) => match self.items.get(h)? {
                Content::Object(o) => Some(o),
                _ => None,
            },
            _ => None,
        }
    }

    /// A field of an object, looked up from the most derived class upward.
    pub fn field(&self, v: Value, name: &str) -> Option<Value> {
        let o = self.object(v)?;
        for cd in o.data.iter().rev() {
            let desc = self.desc(cd.desc)?;
            if let Some(i) = desc.fields.iter().position(|f| f.name == name) {
                return cd.values.get(i).copied();
            }
        }
        None
    }

    /// The names of every field of an object, with their values.
    pub fn fields(&self, v: Value) -> Vec<(&str, Value)> {
        let mut out = Vec::new();
        if let Some(o) = self.object(v) {
            for cd in &o.data {
                if let Some(desc) = self.desc(cd.desc) {
                    for (f, value) in desc.fields.iter().zip(&cd.values) {
                        out.push((f.name.as_str(), *value));
                    }
                }
            }
        }
        out
    }

    /// The `writeObject` data of the part of an object that belongs to
    /// class `class`.
    pub fn annotation(&self, v: Value, class: &str) -> Option<&[Item]> {
        let o = self.object(v)?;
        o.data
            .iter()
            .find(|cd| self.desc(cd.desc).is_some_and(|d| d.name == class))
            .map(|cd| cd.annotation.as_slice())
    }

    pub fn string(&self, v: Value) -> Option<&str> {
        match v {
            Value::Ref(h) => match self.items.get(h)? {
                Content::String(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn enum_name(&self, v: Value) -> Option<&str> {
        match v {
            Value::Ref(h) => match self.items.get(h)? {
                Content::Enum { name, .. } => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    /// An integer, primitive or boxed (`java.lang.Integer` and friends).
    pub fn int(&self, v: Value) -> Option<i64> {
        match v {
            Value::Byte(b) => Some(b as i64),
            Value::Short(s) => Some(s as i64),
            Value::Char(c) => Some(c as i64),
            Value::Int(i) => Some(i as i64),
            Value::Long(l) => Some(l),
            Value::Ref(_) => match self.class_name(v)? {
                "java.lang.Integer" | "java.lang.Long" | "java.lang.Short" | "java.lang.Byte" => {
                    self.int(self.field(v, "value")?)
                }
                _ => None,
            },
            _ => None,
        }
    }

    pub fn float(&self, v: Value) -> Option<f64> {
        match v {
            Value::Float(f) => Some(f as f64),
            Value::Double(d) => Some(d),
            Value::Ref(_) => match self.class_name(v)? {
                "java.lang.Float" | "java.lang.Double" => self.float(self.field(v, "value")?),
                _ => self.int(v).map(|i| i as f64),
            },
            other => self.int(other).map(|i| i as f64),
        }
    }

    pub fn bool(&self, v: Value) -> Option<bool> {
        match v {
            Value::Bool(b) => Some(b),
            Value::Ref(_) if self.class_name(v) == Some("java.lang.Boolean") => self.bool(self.field(v, "value")?),
            _ => None,
        }
    }

    pub fn array(&self, v: Value) -> Option<&Array> {
        match v {
            Value::Ref(h) => match self.items.get(h)? {
                Content::Array { data, .. } => Some(data),
                _ => None,
            },
            _ => None,
        }
    }

    /// The entries of a map (`HashMap`, `LinkedHashMap`, `TreeMap`,
    /// `Hashtable`, `EnumMap`, unmodifiable and synchronized wrappers...).
    pub fn map_entries(&self, v: Value) -> Option<Vec<(Value, Value)>> {
        let class = self.class_name(v)?;
        match class {
            "java.util.Collections$UnmodifiableMap"
            | "java.util.Collections$UnmodifiableSortedMap"
            | "java.util.Collections$UnmodifiableNavigableMap"
            | "java.util.Collections$SynchronizedMap"
            | "java.util.Collections$SynchronizedSortedMap"
            | "java.util.Collections$CheckedMap" => self.map_entries(self.field(v, "m")?),
            "java.util.Collections$EmptyMap" => Some(Vec::new()),
            "com.google.common.collect.ImmutableMap$SerializedForm"
            | "com.google.common.collect.ImmutableBiMap$SerializedForm"
            | "com.google.common.collect.ImmutableSortedMap$SerializedForm"
            | "com.google.common.collect.ImmutableEnumMap$EnumSerializedForm" => {
                if let Some(delegate) = self.field(v, "delegate") {
                    return self.map_entries(delegate);
                }
                let keys = self.list_items(self.field(v, "keys")?)?;
                let values = self.list_items(self.field(v, "values")?)?;
                Some(keys.into_iter().zip(values).collect())
            }
            "java.util.Collections$SingletonMap" => Some(vec![(self.field(v, "k")?, self.field(v, "v")?)]),
            _ => {
                let root = match class {
                    "java.util.LinkedHashMap" | "java.util.HashMap" => "java.util.HashMap",
                    "java.util.TreeMap" => "java.util.TreeMap",
                    "java.util.Hashtable" | "java.util.Properties" => "java.util.Hashtable",
                    "java.util.EnumMap" => "java.util.EnumMap",
                    "java.util.IdentityHashMap" => "java.util.IdentityHashMap",
                    "java.util.WeakHashMap" => return None,
                    "java.util.concurrent.ConcurrentHashMap" => "java.util.concurrent.ConcurrentHashMap",
                    _ => return None,
                };
                let values: Vec<Value> = self
                    .annotation(v, root)?
                    .iter()
                    .filter_map(|i| match i {
                        Item::Value(v) => Some(*v),
                        Item::Block(_) => None,
                    })
                    .collect();
                Some(
                    values
                        .as_chunks::<2>().0.iter()
                        .map(|p| (p[0], p[1]))
                        .filter(|(k, v)| !(*k == Value::Null && *v == Value::Null))
                        .collect(),
                )
            }
        }
    }

    /// The elements of a collection (`ArrayList`, `LinkedList`, `HashSet`,
    /// `TreeSet`, `EnumSet`, `Arrays.asList`, wrappers...) or of an object
    /// array.
    pub fn list_items(&self, v: Value) -> Option<Vec<Value>> {
        if let Some(Array::Object(items)) = self.array(v) {
            return Some(items.clone());
        }
        let class = self.class_name(v)?;
        let from_annotation = |root: &str, skip_values: usize| -> Option<Vec<Value>> {
            Some(
                self.annotation(v, root)?
                    .iter()
                    .filter_map(|i| match i {
                        Item::Value(v) => Some(*v),
                        Item::Block(_) => None,
                    })
                    .skip(skip_values)
                    .collect(),
            )
        };
        match class {
            "java.util.ArrayList" => from_annotation("java.util.ArrayList", 0),
            "java.util.LinkedList" => from_annotation("java.util.LinkedList", 0),
            "java.util.HashSet" | "java.util.LinkedHashSet" => from_annotation("java.util.HashSet", 0),
            "java.util.TreeSet" => from_annotation("java.util.TreeSet", 1),
            "java.util.concurrent.CopyOnWriteArrayList" => from_annotation(class, 0),
            "java.util.ArrayDeque" => from_annotation(class, 0),
            "java.util.Vector" => {
                let n = self.int(self.field(v, "elementCount")?)? as usize;
                let mut items = self.list_items(self.field(v, "elementData")?)?;
                items.truncate(n);
                Some(items)
            }
            "java.util.EnumSet$SerializationProxy" => self.list_items(self.field(v, "elements")?),
            "java.util.Arrays$ArrayList" => self.list_items(self.field(v, "a")?),
            "java.util.Collections$UnmodifiableCollection"
            | "java.util.Collections$UnmodifiableList"
            | "java.util.Collections$UnmodifiableRandomAccessList"
            | "java.util.Collections$UnmodifiableSet"
            | "java.util.Collections$UnmodifiableSortedSet"
            | "java.util.Collections$SynchronizedCollection"
            | "java.util.Collections$SynchronizedList"
            | "java.util.Collections$SynchronizedRandomAccessList"
            | "java.util.Collections$SynchronizedSet" => self.list_items(self.field(v, "c")?),
            "java.util.Collections$EmptyList" | "java.util.Collections$EmptySet" => Some(Vec::new()),
            "com.google.common.collect.ImmutableList$SerializedForm"
            | "com.google.common.collect.ImmutableSet$SerializedForm"
            | "com.google.common.collect.ImmutableSortedSet$SerializedForm" => self.list_items(self.field(v, "elements")?),
            "com.google.common.collect.ImmutableEnumSet$EnumSerializedForm" => self.list_items(self.field(v, "delegate")?),
            "java.util.Collections$SingletonList" | "java.util.Collections$SingletonSet" => {
                Some(vec![self.field(v, "element")?])
            }
            _ => None,
        }
    }
}

/// Called when an object has been read completely.
pub type Hook<'a> = Box<dyn FnMut(&mut Arena, Handle) + 'a>;

/// Reads a stream object by object.
pub struct Parser<'a, R: Read> {
    r: R,
    pub arena: Arena,
    /// Wire handle (minus the base) → arena index. Cleared by `TC_RESET`.
    handles: Vec<Handle>,
    hook: Option<Hook<'a>>,
    depth: usize,
    /// Bytes read so far.
    pub position: u64,
}

impl<'a, R: Read> Parser<'a, R> {
    pub fn new(mut r: R) -> Result<Self> {
        let mut head = [0u8; 4];
        r.read_exact(&mut head).map_err(|_| JavaError::NotJava)?;
        if u16::from_be_bytes([head[0], head[1]]) != MAGIC || u16::from_be_bytes([head[2], head[3]]) != VERSION {
            return Err(JavaError::NotJava);
        }
        Ok(Self {
            r,
            arena: Arena::default(),
            handles: Vec::new(),
            hook: None,
            depth: 0,
            position: 4,
        })
    }

    /// Be told about every object as soon as it is complete.
    pub fn set_hook(&mut self, hook: Hook<'a>) {
        self.hook = Some(hook);
    }

    // ------------------------------------------------------------- bytes --

    fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        // Read in pieces so that a corrupt length cannot allocate gigabytes.
        let mut left = n;
        while left > 0 {
            let chunk = left.min(1 << 20);
            let at = buf.len();
            buf.resize(at + chunk, 0);
            self.r.read_exact(&mut buf[at..])?;
            left -= chunk;
        }
        self.position += n as u64;
        Ok(buf)
    }

    fn u8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.r.read_exact(&mut b)?;
        self.position += 1;
        Ok(b[0])
    }

    fn array_of<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b)?;
        self.position += N as u64;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array_of()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array_of()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array_of()?))
    }

    fn utf(&mut self, len: usize) -> Result<String> {
        let raw = self.bytes(len)?;
        Ok(decode_modified_utf8(&raw))
    }

    fn short_utf(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        self.utf(len)
    }

    // ----------------------------------------------------------- handles --

    fn new_handle(&mut self, content: Content) -> Handle {
        let idx = self.arena.items.len();
        self.arena.items.push(content);
        self.handles.push(idx);
        idx
    }

    fn reference(&mut self) -> Result<Handle> {
        let wire = self.i32()?;
        let i = wire.wrapping_sub(BASE_HANDLE);
        if i < 0 || i as usize >= self.handles.len() {
            return corrupt(format!("reference to unknown handle {wire:#x}"));
        }
        Ok(self.handles[i as usize])
    }

    // ----------------------------------------------------------- objects --

    /// The next top-level object, or `None` at the end of the stream.
    pub fn next_object(&mut self) -> Result<Option<Value>> {
        loop {
            let mut tag = [0u8; 1];
            match self.r.read(&mut tag) {
                Ok(0) => return Ok(None),
                Ok(_) => self.position += 1,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
            match tag[0] {
                TC_BLOCKDATA => {
                    let n = self.u8()? as usize;
                    self.bytes(n)?;
                }
                TC_BLOCKDATALONG => {
                    let n = self.i32()?;
                    if n < 0 {
                        return corrupt("negative block length");
                    }
                    self.bytes(n as usize)?;
                }
                t => return self.object_tagged(t).map(Some),
            }
        }
    }

    fn object(&mut self) -> Result<Value> {
        let tag = self.u8()?;
        self.object_tagged(tag)
    }

    fn object_tagged(&mut self, tag: u8) -> Result<Value> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return corrupt("objects nested too deeply");
        }
        let result = self.object_inner(tag);
        self.depth -= 1;
        result
    }

    fn object_inner(&mut self, tag: u8) -> Result<Value> {
        match tag {
            TC_NULL => Ok(Value::Null),
            TC_REFERENCE => Ok(Value::Ref(self.reference()?)),
            TC_CLASSDESC | TC_PROXYCLASSDESC => {
                let h = self.class_desc_tagged(tag)?;
                Ok(h.map_or(Value::Null, Value::Ref))
            }
            TC_OBJECT => self.new_object(),
            TC_STRING => {
                let h = self.new_handle(Content::Pending);
                let s = self.short_utf()?;
                self.arena.items[h] = Content::String(s);
                Ok(Value::Ref(h))
            }
            TC_LONGSTRING => {
                let h = self.new_handle(Content::Pending);
                let len = self.i64()?;
                if !(0..=i32::MAX as i64).contains(&len) {
                    return corrupt("absurd string length");
                }
                let s = self.utf(len as usize)?;
                self.arena.items[h] = Content::String(s);
                Ok(Value::Ref(h))
            }
            TC_ARRAY => self.new_array(),
            TC_CLASS => {
                let desc = self.class_desc()?.unwrap_or(usize::MAX);
                Ok(Value::Ref(self.new_handle(Content::Class(desc))))
            }
            TC_ENUM => {
                let Some(desc) = self.class_desc()? else {
                    return corrupt("enum without a class");
                };
                let h = self.new_handle(Content::Pending);
                let name = self.object()?;
                let name = self.arena.string(name).map(str::to_owned);
                let Some(name) = name else {
                    return corrupt("enum constant without a name");
                };
                self.arena.items[h] = Content::Enum { desc, name };
                Ok(Value::Ref(h))
            }
            TC_RESET => {
                self.handles.clear();
                self.object()
            }
            TC_EXCEPTION => Err(JavaError::Exception),
            other => corrupt(format!("unexpected tag {other:#04x} at byte {}", self.position - 1)),
        }
    }

    fn class_desc(&mut self) -> Result<Option<Handle>> {
        let tag = self.u8()?;
        self.class_desc_tagged(tag)
    }

    fn class_desc_tagged(&mut self, tag: u8) -> Result<Option<Handle>> {
        match tag {
            TC_NULL => Ok(None),
            TC_REFERENCE => {
                let h = self.reference()?;
                match self.arena.items.get(h) {
                    Some(Content::ClassDesc(_)) | Some(Content::Pending) => Ok(Some(h)),
                    _ => corrupt("class descriptor reference to something else"),
                }
            }
            TC_CLASSDESC => {
                let name = self.short_utf()?;
                let suid = self.i64()?;
                let h = self.new_handle(Content::Pending);
                let flags = self.u8()?;
                let count = self.u16()? as usize;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    let type_code = self.u8()?;
                    let field_name = self.short_utf()?;
                    let class_name = match type_code {
                        b'L' | b'[' => {
                            let v = self.object()?;
                            self.arena.string(v).map(str::to_owned)
                        }
                        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' => None,
                        t => return corrupt(format!("unknown field type {t:#04x} in {name}")),
                    };
                    fields.push(Field {
                        name: field_name,
                        type_code,
                        class_name,
                    });
                }
                self.annotation()?;
                let super_class = self.class_desc()?;
                self.arena.items[h] = Content::ClassDesc(ClassDesc {
                    name,
                    suid,
                    flags,
                    fields,
                    super_class,
                });
                Ok(Some(h))
            }
            TC_PROXYCLASSDESC => {
                let h = self.new_handle(Content::Pending);
                let count = self.i32()?;
                if !(0..=65535).contains(&count) {
                    return corrupt("absurd proxy interface count");
                }
                let mut interfaces = Vec::new();
                for _ in 0..count {
                    interfaces.push(self.short_utf()?);
                }
                self.annotation()?;
                let super_class = self.class_desc()?;
                self.arena.items[h] = Content::ClassDesc(ClassDesc {
                    name: format!("$Proxy({})", interfaces.join(", ")),
                    suid: 0,
                    flags: SC_SERIALIZABLE,
                    fields: Vec::new(),
                    super_class,
                });
                Ok(Some(h))
            }
            other => corrupt(format!("expected a class descriptor, found tag {other:#04x}")),
        }
    }

    /// Block data and objects up to `TC_ENDBLOCKDATA`.
    fn annotation(&mut self) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        loop {
            match self.u8()? {
                TC_ENDBLOCKDATA => return Ok(items),
                TC_BLOCKDATA => {
                    let n = self.u8()? as usize;
                    let b = self.bytes(n)?;
                    push_block(&mut items, b);
                }
                TC_BLOCKDATALONG => {
                    let n = self.i32()?;
                    if n < 0 {
                        return corrupt("negative block length");
                    }
                    let b = self.bytes(n as usize)?;
                    push_block(&mut items, b);
                }
                tag => {
                    let v = self.object_tagged(tag)?;
                    items.push(Item::Value(v));
                }
            }
        }
    }

    fn hierarchy(&self, desc: Handle) -> Result<Vec<Handle>> {
        let mut chain = Vec::new();
        let mut at = Some(desc);
        while let Some(h) = at {
            let Some(Content::ClassDesc(d)) = self.arena.items.get(h) else {
                return corrupt("broken class hierarchy");
            };
            chain.push(h);
            at = d.super_class;
            if chain.len() > 256 {
                return corrupt("class hierarchy loops");
            }
        }
        chain.reverse();
        Ok(chain)
    }

    fn new_object(&mut self) -> Result<Value> {
        let Some(desc) = self.class_desc()? else {
            return corrupt("object without a class");
        };
        let h = self.new_handle(Content::Pending);
        let mut data = Vec::new();
        for class in self.hierarchy(desc)? {
            let Some(Content::ClassDesc(d)) = self.arena.items.get(class) else {
                unreachable!("checked by hierarchy()")
            };
            let (flags, fields) = (d.flags, d.fields.iter().map(|f| f.type_code).collect::<Vec<u8>>());
            let mut cd = ClassData {
                desc: class,
                values: Vec::new(),
                annotation: Vec::new(),
            };
            if flags & SC_EXTERNALIZABLE != 0 {
                if flags & SC_BLOCK_DATA == 0 {
                    let name = match self.arena.items.get(class) {
                        Some(Content::ClassDesc(d)) => d.name.clone(),
                        _ => String::new(),
                    };
                    return Err(JavaError::Externalizable(name));
                }
                cd.annotation = self.annotation()?;
            } else if flags & SC_SERIALIZABLE != 0 {
                cd.values.reserve(fields.len());
                for t in fields {
                    let v = match t {
                        b'B' => Value::Byte(self.u8()? as i8),
                        b'C' => Value::Char(self.u16()?),
                        b'D' => Value::Double(f64::from_bits(self.i64()? as u64)),
                        b'F' => Value::Float(f32::from_bits(self.i32()? as u32)),
                        b'I' => Value::Int(self.i32()?),
                        b'J' => Value::Long(self.i64()?),
                        b'S' => Value::Short(self.u16()? as i16),
                        b'Z' => Value::Bool(self.u8()? != 0),
                        _ => self.object()?,
                    };
                    cd.values.push(v);
                }
                if flags & SC_WRITE_METHOD != 0 {
                    cd.annotation = self.annotation()?;
                }
            }
            data.push(cd);
        }
        self.arena.items[h] = Content::Object(Object { desc, data });
        if let Some(hook) = &mut self.hook {
            hook(&mut self.arena, h);
        }
        Ok(Value::Ref(h))
    }

    fn new_array(&mut self) -> Result<Value> {
        let Some(desc) = self.class_desc()? else {
            return corrupt("array without a class");
        };
        let name = match self.arena.items.get(desc) {
            Some(Content::ClassDesc(d)) => d.name.clone(),
            _ => return corrupt("array class is not a descriptor"),
        };
        let h = self.new_handle(Content::Pending);
        let n = self.i32()?;
        if n < 0 {
            return corrupt("negative array length");
        }
        let n = n as usize;
        let element = name.as_bytes().get(1).copied().unwrap_or(0);
        let data = match element {
            b'B' => Array::Byte(self.bytes(n)?),
            b'Z' => Array::Bool(self.bytes(n)?.into_iter().map(|b| b != 0).collect()),
            b'C' => Array::Char(self.words::<2>(n)?.map(u16::from_be_bytes).collect()),
            b'S' => Array::Short(self.words::<2>(n)?.map(i16::from_be_bytes).collect()),
            b'I' => Array::Int(self.words::<4>(n)?.map(i32::from_be_bytes).collect()),
            b'F' => Array::Float(self.words::<4>(n)?.map(|w| f32::from_bits(u32::from_be_bytes(w))).collect()),
            b'J' => Array::Long(self.words::<8>(n)?.map(i64::from_be_bytes).collect()),
            b'D' => Array::Double(self.words::<8>(n)?.map(|w| f64::from_bits(u64::from_be_bytes(w))).collect()),
            b'L' | b'[' => {
                let mut items = Vec::with_capacity(n.min(1 << 16));
                for _ in 0..n {
                    items.push(self.object()?);
                }
                Array::Object(items)
            }
            _ => return corrupt(format!("unknown array type {name}")),
        };
        self.arena.items[h] = Content::Array { desc, data };
        Ok(Value::Ref(h))
    }

    fn words<const N: usize>(&mut self, n: usize) -> Result<impl Iterator<Item = [u8; N]>> {
        let raw = self.bytes(n.checked_mul(N).ok_or(JavaError::Corrupt("array too large".into()))?)?;
        Ok((0..n).map(move |i| raw[i * N..i * N + N].try_into().unwrap()))
    }
}

fn push_block(items: &mut Vec<Item>, b: Vec<u8>) {
    // Consecutive blocks are one logical run of data.
    if let Some(Item::Block(prev)) = items.last_mut() {
        prev.extend_from_slice(&b);
    } else {
        items.push(Item::Block(b));
    }
}

/// Java's "modified UTF-8": NUL is two bytes and supplementary characters
/// are surrogate pairs of three bytes each.
pub fn decode_modified_utf8(raw: &[u8]) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b & 0x80 == 0 {
            units.push(b as u16);
            i += 1;
        } else if b & 0xE0 == 0xC0 && i + 1 < raw.len() {
            units.push((((b & 0x1F) as u16) << 6) | (raw[i + 1] & 0x3F) as u16);
            i += 2;
        } else if b & 0xF0 == 0xE0 && i + 2 < raw.len() {
            units.push((((b & 0x0F) as u16) << 12) | (((raw[i + 1] & 0x3F) as u16) << 6) | (raw[i + 2] & 0x3F) as u16);
            i += 3;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds streams by hand, byte for byte, the way `ObjectOutputStream`
    /// would.
    #[derive(Default)]
    pub struct StreamWriter {
        pub out: Vec<u8>,
        /// The handle the next new item will get.
        pub next: i32,
    }

    impl StreamWriter {
        pub fn new() -> Self {
            let mut out = Vec::new();
            out.extend_from_slice(&MAGIC.to_be_bytes());
            out.extend_from_slice(&VERSION.to_be_bytes());
            Self { out, next: 0 }
        }
        /// Account for the handle of an object or array whose class was
        /// just written; returns it.
        pub fn handle(&mut self) -> i32 {
            self.next += 1;
            self.next - 1
        }
        pub fn u8(&mut self, b: u8) -> &mut Self {
            self.out.push(b);
            self
        }
        pub fn u16(&mut self, v: u16) -> &mut Self {
            self.out.extend_from_slice(&v.to_be_bytes());
            self
        }
        pub fn i32(&mut self, v: i32) -> &mut Self {
            self.out.extend_from_slice(&v.to_be_bytes());
            self
        }
        pub fn i64(&mut self, v: i64) -> &mut Self {
            self.out.extend_from_slice(&v.to_be_bytes());
            self
        }
        pub fn utf(&mut self, s: &str) -> &mut Self {
            self.u16(s.len() as u16);
            self.out.extend_from_slice(s.as_bytes());
            self
        }
        pub fn string(&mut self, s: &str) -> &mut Self {
            self.next += 1;
            self.u8(TC_STRING).utf(s)
        }
        /// A class descriptor with primitive (`I`, `Z`...) or object
        /// (`L`, with a type string) fields and the given super class
        /// written by `superclass`.
        pub fn class(&mut self, name: &str, flags: u8, fields: &[(u8, &str, Option<&str>)], superclass: impl FnOnce(&mut Self)) -> &mut Self {
            self.u8(TC_CLASSDESC).utf(name).i64(42);
            self.next += 1;
            self.u8(flags).u16(fields.len() as u16);
            for (t, n, class) in fields {
                self.u8(*t).utf(n);
                if let Some(c) = class {
                    self.string(c);
                }
            }
            self.u8(TC_ENDBLOCKDATA);
            superclass(self);
            self
        }
        pub fn reference(&mut self, handle: i32) -> &mut Self {
            self.u8(TC_REFERENCE).i32(BASE_HANDLE + handle)
        }
    }

    #[test]
    fn modified_utf8_decodes_nul_and_surrogates() {
        assert_eq!(decode_modified_utf8(b"abc"), "abc");
        assert_eq!(decode_modified_utf8(&[0xC0, 0x80]), "\0");
        assert_eq!(decode_modified_utf8("é€".as_bytes()), "é€");
        assert_eq!(decode_modified_utf8(&[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]), "😀");
    }

    #[test]
    fn objects_arrays_strings_enums_and_references() {
        let mut w = StreamWriter::new();
        // handle 0: class desc Point; handle 1: the object.
        w.u8(TC_OBJECT)
            .class("java.awt.Point", SC_SERIALIZABLE, &[(b'I', "x", None), (b'I', "y", None)], |w| {
                w.u8(TC_NULL);
            })
            .i32(-3)
            .i32(7);
        // handle 2: array class [S; handle 3: the array.
        w.u8(TC_ARRAY).class("[S", SC_SERIALIZABLE, &[], |w| {
            w.u8(TC_NULL);
        });
        w.i32(3).u16(1).u16(0xFFFF).u16(256);
        // handle 4: a string; then a reference back to it.
        w.string("héllo");
        w.reference(4);
        // handle 5: enum class desc whose super is java.lang.Enum (handle 6).
        w.u8(TC_ENUM)
            .class("org.example.Kind", SC_SERIALIZABLE | 0x10, &[], |w| {
                w.class("java.lang.Enum", SC_SERIALIZABLE | 0x10, &[], |w| {
                    w.u8(TC_NULL);
                });
            })
            .string("SOLID");
        // A second Point reusing the class descriptor by reference.
        w.u8(TC_OBJECT).reference(0).i32(1).i32(2);

        let mut p = Parser::new(&w.out[..]).unwrap();
        let mut values = Vec::new();
        while let Some(v) = p.next_object().unwrap() {
            values.push(v);
        }
        let a = &p.arena;
        assert_eq!(values.len(), 6);
        assert_eq!(a.class_name(values[0]), Some("java.awt.Point"));
        assert_eq!(a.int(a.field(values[0], "x").unwrap()), Some(-3));
        assert_eq!(a.int(a.field(values[0], "y").unwrap()), Some(7));
        match a.array(values[1]) {
            Some(Array::Short(s)) => assert_eq!(s, &vec![1, -1, 256]),
            other => panic!("{other:?}"),
        }
        assert_eq!(a.string(values[2]), Some("héllo"));
        assert_eq!(values[3], values[2]);
        assert_eq!(a.enum_name(values[4]), Some("SOLID"));
        assert_eq!(a.class_name(values[4]), Some("org.example.Kind"));
        assert_eq!(a.int(a.field(values[5], "y").unwrap()), Some(2));
    }

    #[test]
    fn hashmaps_and_lists_are_read_from_their_write_object_data() {
        let mut w = StreamWriter::new();
        // HashMap { "a" -> Integer(1), "b" -> null } with its writeObject data.
        w.u8(TC_OBJECT).class(
            "java.util.HashMap",
            SC_SERIALIZABLE | SC_WRITE_METHOD,
            &[(b'F', "loadFactor", None), (b'I', "threshold", None)],
            |w| {
                w.u8(TC_NULL);
            },
        );
        w.i32(0x3F40_0000).i32(12); // fields
        w.u8(TC_BLOCKDATA).u8(8).i32(16).i32(2); // capacity, size
        w.string("a");
        w.u8(TC_OBJECT)
            .class("java.lang.Integer", SC_SERIALIZABLE, &[(b'I', "value", None)], |w| {
                w.class("java.lang.Number", SC_SERIALIZABLE, &[], |w| {
                    w.u8(TC_NULL);
                });
            })
            .i32(1);
        w.string("b").u8(TC_NULL);
        w.u8(TC_ENDBLOCKDATA);
        // ArrayList ["x"]
        w.u8(TC_OBJECT).class("java.util.ArrayList", SC_SERIALIZABLE | SC_WRITE_METHOD, &[(b'I', "size", None)], |w| {
            w.u8(TC_NULL);
        });
        w.i32(1).u8(TC_BLOCKDATA).u8(4).i32(1).string("x").u8(TC_ENDBLOCKDATA);

        let mut p = Parser::new(&w.out[..]).unwrap();
        let map = p.next_object().unwrap().unwrap();
        let list = p.next_object().unwrap().unwrap();
        assert!(p.next_object().unwrap().is_none());
        let a = &p.arena;
        let entries = a.map_entries(map).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(a.string(entries[0].0), Some("a"));
        assert_eq!(a.int(entries[0].1), Some(1));
        assert_eq!(entries[1].1, Value::Null);
        let items = a.list_items(list).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(a.string(items[0]), Some("x"));
    }

    #[test]
    fn hooks_run_as_objects_complete_and_can_free_them() {
        let mut w = StreamWriter::new();
        for i in 0..3 {
            w.u8(TC_OBJECT);
            if i == 0 {
                w.class("T", SC_SERIALIZABLE, &[(b'I', "v", None)], |w| {
                    w.u8(TC_NULL);
                });
            } else {
                w.reference(0);
            }
            w.i32(i * 10);
        }
        let mut seen = Vec::new();
        let mut p = Parser::new(&w.out[..]).unwrap();
        p.set_hook(Box::new(|arena: &mut Arena, h: Handle| {
            let v = Value::Ref(h);
            if arena.class_name(v) == Some("T") {
                seen.push(arena.int(arena.field(v, "v").unwrap()).unwrap());
                arena.take(h);
            }
        }));
        while p.next_object().unwrap().is_some() {}
        let freed = p.arena.items.iter().filter(|c| matches!(c, Content::Freed)).count();
        drop(p);
        assert_eq!(seen, vec![0, 10, 20]);
        assert_eq!(freed, 3);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(matches!(Parser::new(&b"\x1f\x8b\x08\x00"[..]), Err(JavaError::NotJava)));
        let mut w = StreamWriter::new();
        w.u8(0x42);
        let mut p = Parser::new(&w.out[..]).unwrap();
        assert!(matches!(p.next_object(), Err(JavaError::Corrupt(_))));
        let mut w = StreamWriter::new();
        w.reference(99);
        let mut p = Parser::new(&w.out[..]).unwrap();
        assert!(p.next_object().is_err());
        // Truncated array.
        let mut w = StreamWriter::new();
        w.u8(TC_ARRAY).class("[I", SC_SERIALIZABLE, &[], |w| {
            w.u8(TC_NULL);
        });
        w.i32(1000).i32(1);
        let mut p = Parser::new(&w.out[..]).unwrap();
        assert!(matches!(p.next_object(), Err(JavaError::Io(_))));
    }
}
