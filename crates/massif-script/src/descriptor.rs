//! A script's descriptor, as WorldPainter reads it: `key=value` comments
//! before the first line of code naming the script, describing it and its
//! parameters, which the Run script dialog asks for.
//!
//! ```text
//! // script.name=Rivers
//! // script.description=Carves the rivers\nfrom a mask
//! // script.param.mask.type=file
//! // script.param.mask.description=The rivers, white on black
//! // script.param.depth.type=integer
//! // script.param.depth.default=4
//! // script.hideCmdLineParams=true
//! ```

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    String,
    Integer,
    Percentage,
    Float,
    File,
    Boolean,
}

impl ParamKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "string" => Self::String,
            "integer" => Self::Integer,
            "percentage" => Self::Percentage,
            "float" => Self::Float,
            "file" => Self::File,
            "boolean" => Self::Boolean,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub kind: ParamKind,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub default: Option<String>,
    pub optional: bool,
}

impl Param {
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

/// A parameter's value, as the dialog or the command line gives it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    File(Option<std::path::PathBuf>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Descriptor {
    pub name: Option<String>,
    pub description: Option<String>,
    pub params: Vec<Param>,
    pub hide_command_line: bool,
}

impl Descriptor {
    /// The descriptor at the head of `source`: its comment lines up to the
    /// first that is neither empty nor a comment.
    pub fn parse(source: &str) -> Self {
        let mut d = Descriptor::default();
        for line in source.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let Some(body) = t.strip_prefix("//") else { break };
            let Some((key, value)) = body.trim().split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim().replace("\\n", "\n"));
            match key {
                "script.name" => d.name = Some(value),
                "script.description" => d.description = Some(value),
                "script.hideCmdLineParams" => d.hide_command_line = value.eq_ignore_ascii_case("true"),
                _ => {
                    let Some(rest) = key.strip_prefix("script.param.") else { continue };
                    let Some((name, prop)) = rest.rsplit_once('.') else { continue };
                    let k = match d.params.iter().position(|p| p.name == name) {
                        Some(k) => k,
                        None => {
                            d.params.push(Param { name: name.to_owned(), kind: ParamKind::String, description: None, display_name: None, default: None, optional: false });
                            d.params.len() - 1
                        }
                    };
                    let p = &mut d.params[k];
                    match prop {
                        "type" => p.kind = ParamKind::parse(&value).unwrap_or(ParamKind::String),
                        "description" => p.description = Some(value),
                        "displayName" => p.display_name = Some(value),
                        "default" => p.default = Some(value),
                        "optional" => p.optional = value.eq_ignore_ascii_case("true"),
                        _ => {}
                    }
                }
            }
        }
        d
    }

    /// The default value of each parameter, as typed in the dialog first.
    pub fn defaults(&self) -> Vec<(String, Value)> {
        self.params.iter().map(|p| (p.name.clone(), p.parse(p.default.as_deref().unwrap_or("")))).collect()
    }
}

impl Param {
    /// `text` as this parameter's value: what cannot be read is the
    /// type's zero, as WorldPainter's dialog leaves it.
    pub fn parse(&self, text: &str) -> Value {
        let t = text.trim();
        match self.kind {
            ParamKind::String => Value::String(text.to_owned()),
            ParamKind::Integer | ParamKind::Percentage => Value::Integer(t.parse::<f64>().map(|v| v.round() as i64).unwrap_or(0)),
            ParamKind::Float => Value::Float(t.parse().unwrap_or(0.0)),
            ParamKind::Boolean => Value::Boolean(t.eq_ignore_ascii_case("true")),
            ParamKind::File => Value::File((!t.is_empty()).then(|| t.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WorldPainter's own descriptor test script reads as WorldPainter reads it.
    #[test]
    fn reads_worldpainters_descriptor_test() {
        let src = "// -- This is the script descriptor.\n//\n// script.name=Test\n// script.description=Description of test\\nwith optional line endings\n// script.param.firstParam.type=integer\n// script.param.firstParam.description=Description of firstParam. Will be shown as tooltip\n// script.param.anotherParam.type=percentage\n// script.param.anotherParam.default=50\n// script.param.file1.type=file\n// script.param.file1.optional=true\n// script.param.demoParam.type=boolean\n// script.param.demoParam.displayName=Demo parameter\n// script.param.demoParam.default=true\n// script.hideCmdLineParams=true\n\nvar firstParam = params['firstParam'];\n// script.name=Not this one\n";
        let d = Descriptor::parse(src);
        assert_eq!(d.name.as_deref(), Some("Test"));
        assert_eq!(d.description.as_deref(), Some("Description of test\nwith optional line endings"));
        assert!(d.hide_command_line);
        assert_eq!(d.params.len(), 4);
        assert_eq!(d.params[0].kind, ParamKind::Integer);
        assert_eq!(d.params[1].default.as_deref(), Some("50"));
        assert!(d.params[2].optional);
        assert_eq!(d.params[3].label(), "Demo parameter");
        assert_eq!(d.defaults()[3].1, Value::Boolean(true));
        assert_eq!(d.defaults()[1].1, Value::Integer(50));
    }
}
