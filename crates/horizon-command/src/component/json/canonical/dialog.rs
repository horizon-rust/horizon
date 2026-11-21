//! Encoders paired with the pinned inline Dialog codecs. Nested items use the
//! caller's borrowed payload encoder, while plain controls need no world.
use super::*;
mod input;

pub(super) fn encode(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    super::super::dialog::validate(v, ctx)?;
    let m = v.as_object().unwrap();
    let kind = crate::args::parse_identifier(m["type"].as_str().unwrap()).unwrap();
    let mut r = Fields::new(v);
    r.out.insert("type".into(), Value::String(kind.clone()));
    r.required("title", |v| component(v, ctx))?;
    r.optional("external_title", |v| component(v, ctx), None)?;
    for key in ["can_close_with_escape", "pause"] {
        r.optional(key, cloned, Some(Value::Bool(true)))?;
    }
    r.optional("after_action", cloned, Some(Value::String("close".into())))?;
    r.optional(
        "body",
        |v| compact(v, |v| body(v, ctx)),
        Some(Value::Array(vec![])),
    )?;
    r.optional(
        "inputs",
        |v| array(v, |v| input::encode(v, ctx)),
        Some(Value::Array(vec![])),
    )?;
    match kind.as_str() {
        "minecraft:notice" => {
            // NoticeDialog.DEFAULT_ACTION is GUI_OK, width=150, no action.
            let default = Value::Object(Map::from([(
                "label".into(),
                Value::Object(Map::from([(
                    "translate".into(),
                    Value::String("gui.ok".into()),
                )])),
            )]));
            r.optional("action", |v| button(v, ctx), Some(default))?;
        }
        "minecraft:confirmation" => {
            r.required("yes", |v| button(v, ctx))?;
            r.required("no", |v| button(v, ctx))?;
        }
        "minecraft:multi_action" => {
            r.required("actions", |v| array(v, |v| button(v, ctx)))?;
            r.optional("exit_action", |v| button(v, ctx), None)?;
            r.optional("columns", integer, Some(n(2)))?;
        }
        "minecraft:dialog_list" | "minecraft:server_links" => {
            if kind == "minecraft:dialog_list" {
                // HolderSetCodec(false) uses the same singleton compaction
                // as ExtraCodecs.compactListCodec.
                r.required("dialogs", |v| compact(v, |v| encode(v, ctx)))?;
            }
            r.optional("exit_action", |v| button(v, ctx), None)?;
            r.optional("columns", integer, Some(n(2)))?;
            r.optional("button_width", integer, Some(n(150)))?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(r.finish())
}
fn button(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    let mut r = Fields::new(v);
    r.required("label", |v| component(v, ctx))?;
    r.optional("tooltip", |v| component(v, ctx), None)?;
    r.optional("width", integer, Some(n(150)))?;
    r.optional("action", |v| action(v, ctx), None)?;
    Ok(r.finish())
}
fn action(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let kind = crate::args::parse_identifier(m["type"].as_str().unwrap()).unwrap();
    let mut r = Fields::new(v);
    r.out.insert("type".into(), Value::String(kind.clone()));
    match kind.as_str() {
        "minecraft:dynamic/run_command" => r.required("template", cloned)?,
        "minecraft:dynamic/custom" => {
            r.required("id", identifier)?;
            r.optional("additions", data::nbt, None)?;
        }
        _ => {
            let mut event = m.clone();
            event.insert(
                "action".into(),
                Value::String(
                    kind.strip_prefix("minecraft:")
                        .ok_or(Error::Invalid)?
                        .into(),
                ),
            );
            let Value::Object(mut event) = click(&Value::Object(event), ctx)? else {
                unreachable!()
            };
            event.shift_remove("action");
            r.out.extend(event);
        }
    }
    Ok(r.finish())
}
fn body(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let kind = crate::args::parse_identifier(m["type"].as_str().unwrap()).unwrap();
    let mut r = Fields::new(v);
    r.out.insert("type".into(), Value::String(kind.clone()));
    if kind == "minecraft:plain_message" {
        r.required("contents", |v| component(v, ctx))?;
        r.optional("width", integer, Some(n(200)))?;
    } else {
        r.required("item", |v| {
            let bare;
            let m = if v.is_string() {
                bare = Map::from([("id".into(), v.clone())]);
                &bare
            } else {
                v.as_object().unwrap()
            };
            ctx.encode_item_template(m)
        })?;
        r.optional("description", |v| plain(v, ctx), None)?;
        for key in ["show_decorations", "show_tooltip"] {
            r.optional(key, cloned, Some(Value::Bool(true)))?;
        }
        for key in ["width", "height"] {
            r.optional(key, integer, Some(n(16)))?;
        }
    }
    Ok(r.finish())
}
fn plain(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    if let Some(m) = v.as_object()
        && let Some(contents) = present(m, "contents")
    {
        let candidate = (|| {
            let mut r = Fields::new(v);
            r.out.insert("contents".into(), component(contents, ctx)?);
            if let Some(width) = present(m, "width") {
                let width = super::super::number::checked_int(width)?;
                if !(1..=1024).contains(&width) {
                    return Err(Error::Invalid);
                }
                if width != 200 {
                    r.out.insert("width".into(), n(width));
                }
            }
            Ok(r.finish())
        })();
        match candidate {
            Ok(v) => return Ok(v),
            Err(Error::Invalid) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Value::Object(Map::from([(
        "contents".into(),
        component(v, ctx)?,
    )])))
}
fn compact(v: &Value, mut f: impl FnMut(&Value) -> Result<Value, Error>) -> Result<Value, Error> {
    if let Some(a) = v.as_array() {
        let mut a = a.iter().map(f).collect::<Result<Vec<_>, _>>()?;
        Ok(if a.len() == 1 {
            a.remove(0)
        } else {
            Value::Array(a)
        })
    } else {
        f(v)
    }
}
fn array(v: &Value, f: impl FnMut(&Value) -> Result<Value, Error>) -> Result<Value, Error> {
    Ok(Value::Array(
        v.as_array()
            .unwrap()
            .iter()
            .map(f)
            .collect::<Result<_, _>>()?,
    ))
}
fn integer(v: &Value) -> Result<Value, Error> {
    super::payload::integer(v)
}
fn float(v: &Value) -> Result<Value, Error> {
    super::payload::float(v)
}
fn identifier(v: &Value) -> Result<Value, Error> {
    super::payload::identifier(v)
}
fn cloned(v: &Value) -> Result<Value, Error> {
    Ok(v.clone())
}
fn n(v: i32) -> Value {
    Value::Number(v.to_string())
}
struct Fields<'a> {
    input: &'a Map<String, Value>,
    out: Map<String, Value>,
}
impl<'a> Fields<'a> {
    fn new(v: &'a Value) -> Self {
        Self {
            input: v.as_object().unwrap(),
            out: Map::new(),
        }
    }
    fn required(&mut self, key: &str, f: impl FnOnce(&Value) -> Result<Value, Error>) -> R {
        self.out.insert(key.into(), f(&self.input[key])?);
        Ok(())
    }
    fn optional(
        &mut self,
        key: &str,
        f: impl FnOnce(&Value) -> Result<Value, Error>,
        default: Option<Value>,
    ) -> R {
        if let Some(v) = present(self.input, key) {
            let v = f(v)?;
            if Some(&v) != default.as_ref() {
                self.out.insert(key.into(), v);
            }
        }
        Ok(())
    }
    fn finish(self) -> Value {
        Value::Object(self.out)
    }
}
