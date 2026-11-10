//! A streaming writer for Minecraft's NBT format (big-endian, Java edition).
//!
//! There is no intermediate tree: values are appended to one buffer in the
//! order they are written, which is all an exporter needs and keeps chunk
//! serialisation allocation-free.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TagType {
    End = 0,
    Byte = 1,
    Short = 2,
    Int = 3,
    Long = 4,
    Float = 5,
    Double = 6,
    ByteArray = 7,
    String = 8,
    List = 9,
    Compound = 10,
    IntArray = 11,
    LongArray = 12,
}

/// Writes one NBT document whose root is an unnamed compound.
///
/// Named methods (`int`, `string`, `compound`, ...) write a field of the
/// current compound; `item_*` methods write an element of the current list.
/// The caller is responsible for balancing `compound`/`item_compound` with
/// `end`, and for writing exactly as many items as a list announced.
pub struct NbtWriter {
    buf: Vec<u8>,
}

impl Default for NbtWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl NbtWriter {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        buf.push(TagType::Compound as u8);
        buf.extend_from_slice(&0u16.to_be_bytes()); // empty root name
        Self { buf }
    }

    /// Close the root compound and return the encoded document.
    pub fn finish(mut self) -> Vec<u8> {
        self.buf.push(TagType::End as u8);
        self.buf
    }

    /// Java's "modified UTF-8": NUL is two bytes and characters outside the
    /// BMP are a surrogate pair of two three-byte sequences.
    fn put_str(&mut self, s: &str) {
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0, 0]);
        for c in s.chars() {
            let c = c as u32;
            match c {
                0 => self.buf.extend_from_slice(&[0xC0, 0x80]),
                1..=0x7F => self.buf.push(c as u8),
                0x80..=0x7FF => self
                    .buf
                    .extend_from_slice(&[0xC0 | (c >> 6) as u8, 0x80 | (c & 0x3F) as u8]),
                0x800..=0xFFFF => self.put_3(c),
                _ => {
                    let v = c - 0x1_0000;
                    self.put_3(0xD800 | (v >> 10));
                    self.put_3(0xDC00 | (v & 0x3FF));
                }
            }
            if self.buf.len() - at - 2 > u16::MAX as usize - 6 {
                break; // NBT strings are limited to 65535 bytes
            }
        }
        let len = (self.buf.len() - at - 2) as u16;
        self.buf[at..at + 2].copy_from_slice(&len.to_be_bytes());
    }

    fn put_3(&mut self, c: u32) {
        self.buf.extend_from_slice(&[
            0xE0 | (c >> 12) as u8,
            0x80 | ((c >> 6) & 0x3F) as u8,
            0x80 | (c & 0x3F) as u8,
        ]);
    }

    fn header(&mut self, tag: TagType, name: &str) {
        self.buf.push(tag as u8);
        self.put_str(name);
    }

    pub fn byte(&mut self, name: &str, v: i8) {
        self.header(TagType::Byte, name);
        self.buf.push(v as u8);
    }

    pub fn bool(&mut self, name: &str, v: bool) {
        self.byte(name, v as i8);
    }

    pub fn short(&mut self, name: &str, v: i16) {
        self.header(TagType::Short, name);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn int(&mut self, name: &str, v: i32) {
        self.header(TagType::Int, name);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn long(&mut self, name: &str, v: i64) {
        self.header(TagType::Long, name);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn float(&mut self, name: &str, v: f32) {
        self.header(TagType::Float, name);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn double(&mut self, name: &str, v: f64) {
        self.header(TagType::Double, name);
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn string(&mut self, name: &str, v: &str) {
        self.header(TagType::String, name);
        self.put_str(v);
    }

    pub fn byte_array(&mut self, name: &str, v: &[u8]) {
        self.header(TagType::ByteArray, name);
        self.buf.extend_from_slice(&(v.len() as i32).to_be_bytes());
        self.buf.extend_from_slice(v);
    }

    pub fn int_array(&mut self, name: &str, v: &[i32]) {
        self.header(TagType::IntArray, name);
        self.buf.extend_from_slice(&(v.len() as i32).to_be_bytes());
        for x in v {
            self.buf.extend_from_slice(&x.to_be_bytes());
        }
    }

    pub fn long_array(&mut self, name: &str, v: &[i64]) {
        self.header(TagType::LongArray, name);
        self.buf.extend_from_slice(&(v.len() as i32).to_be_bytes());
        self.buf.reserve(v.len() * 8);
        for x in v {
            self.buf.extend_from_slice(&x.to_be_bytes());
        }
    }

    /// Open a compound field; close it with [`NbtWriter::end`].
    pub fn compound(&mut self, name: &str) {
        self.header(TagType::Compound, name);
    }

    /// Close the innermost open compound.
    pub fn end(&mut self) {
        self.buf.push(TagType::End as u8);
    }

    /// Open a list field of `len` elements of type `element`.
    pub fn list(&mut self, name: &str, element: TagType, len: usize) {
        self.header(TagType::List, name);
        // An empty list is conventionally typed End.
        let element = if len == 0 { TagType::End } else { element };
        self.buf.push(element as u8);
        self.buf.extend_from_slice(&(len as i32).to_be_bytes());
    }

    /// Start a compound list element; close it with [`NbtWriter::end`].
    pub fn item_compound(&mut self) {}

    pub fn item_string(&mut self, v: &str) {
        self.put_str(v);
    }

    pub fn item_int(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn item_double(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn item_float(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::Value;
    use std::collections::HashMap;

    #[test]
    fn every_tag_type_reads_back() {
        let mut w = NbtWriter::new();
        w.byte("byte", -5);
        w.bool("flag", true);
        w.short("short", -300);
        w.int("int", 123_456_789);
        w.long("long", -9_000_000_000);
        w.float("float", 1.5);
        w.double("double", -2.25);
        w.string("string", "héllo wörld");
        w.byte_array("bytes", &[1, 2, 255]);
        w.int_array("ints", &[1, -2, 3]);
        w.long_array("longs", &[i64::MIN, 0, i64::MAX]);
        w.compound("nested");
        w.int("x", 1);
        w.compound("deeper");
        w.string("k", "v");
        w.end();
        w.end();
        w.list("strings", TagType::String, 2);
        w.item_string("a");
        w.item_string("b");
        w.list("doubles", TagType::Double, 3);
        w.item_double(0.5);
        w.item_double(64.0);
        w.item_double(-0.5);
        w.list("compounds", TagType::Compound, 2);
        for i in 0..2 {
            w.item_compound();
            w.int("i", i);
            w.end();
        }
        w.list("empty", TagType::Compound, 0);
        let bytes = w.finish();

        let v: HashMap<String, Value> = fastnbt::from_bytes(&bytes).unwrap();
        assert_eq!(v["byte"], Value::Byte(-5));
        assert_eq!(v["flag"], Value::Byte(1));
        assert_eq!(v["short"], Value::Short(-300));
        assert_eq!(v["int"], Value::Int(123_456_789));
        assert_eq!(v["long"], Value::Long(-9_000_000_000));
        assert_eq!(v["float"], Value::Float(1.5));
        assert_eq!(v["double"], Value::Double(-2.25));
        assert_eq!(v["string"], Value::String("héllo wörld".into()));
        match &v["bytes"] {
            Value::ByteArray(a) => assert_eq!(&a[..], &[1i8, 2, -1]),
            other => panic!("{other:?}"),
        }
        match &v["ints"] {
            Value::IntArray(a) => assert_eq!(&a[..], &[1, -2, 3]),
            other => panic!("{other:?}"),
        }
        match &v["longs"] {
            Value::LongArray(a) => assert_eq!(&a[..], &[i64::MIN, 0, i64::MAX]),
            other => panic!("{other:?}"),
        }
        match &v["nested"] {
            Value::Compound(c) => {
                assert_eq!(c["x"], Value::Int(1));
                match &c["deeper"] {
                    Value::Compound(d) => assert_eq!(d["k"], Value::String("v".into())),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            v["strings"],
            Value::List(vec![Value::String("a".into()), Value::String("b".into())])
        );
        assert_eq!(
            v["doubles"],
            Value::List(vec![Value::Double(0.5), Value::Double(64.0), Value::Double(-0.5)])
        );
        match &v["compounds"] {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                match &items[1] {
                    Value::Compound(c) => assert_eq!(c["i"], Value::Int(1)),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(v["empty"], Value::List(vec![]));
    }

    #[test]
    fn modified_utf8() {
        let mut w = NbtWriter::new();
        w.string("s", "a\0é€😀");
        let bytes = w.finish();
        // Skip: root header (3) + tag (1) + name len (2) + "s" (1) = 7.
        let len = u16::from_be_bytes([bytes[7], bytes[8]]) as usize;
        let payload = &bytes[9..9 + len];
        assert_eq!(
            payload,
            &[
                b'a', 0xC0, 0x80, // NUL as two bytes
                0xC3, 0xA9, // é
                0xE2, 0x82, 0xAC, // €
                0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80, // 😀 as a surrogate pair
            ]
        );
        assert_eq!(bytes[9 + len], TagType::End as u8);
    }
}
