use super::*;

pub(super) fn encode(v: &Value, ctx: &dyn Context) -> Result<Value, Error> {
    let m = v.as_object().unwrap();
    let kind = crate::args::parse_identifier(m["type"].as_str().unwrap()).unwrap();
    let mut r = Fields::new(v);
    r.out.insert("type".into(), Value::String(kind.clone()));
    r.required("key", cloned)?;
    r.required("label", |v| component(v, ctx))?;
    if kind != "minecraft:boolean" {
        r.optional("width", integer, Some(n(200)))?;
    }
    match kind.as_str() {
        "minecraft:boolean" => {
            r.optional("initial", cloned, Some(Value::Bool(false)))?;
            r.optional("on_true", cloned, Some(Value::String("true".into())))?;
            r.optional("on_false", cloned, Some(Value::String("false".into())))?;
        }
        "minecraft:text" => {
            r.optional("label_visible", cloned, Some(Value::Bool(true)))?;
            r.optional("initial", cloned, Some(Value::String(String::new())))?;
            r.optional("max_length", integer, Some(n(32)))?;
            r.optional(
                "multiline",
                |v| {
                    let mut r = Fields::new(v);
                    r.optional("max_lines", integer, None)?;
                    r.optional("height", integer, None)?;
                    Ok(r.finish())
                },
                None,
            )?;
        }
        "minecraft:single_option" => {
            r.optional("label_visible", cloned, Some(Value::Bool(true)))?;
            r.required("options", |v| {
                array(v, |v| {
                    if v.is_string() {
                        Ok(Value::Object(Map::from([("id".into(), v.clone())])))
                    } else {
                        let mut r = Fields::new(v);
                        r.required("id", cloned)?;
                        r.optional("display", |v| component(v, ctx), None)?;
                        r.optional("initial", cloned, Some(Value::Bool(false)))?;
                        Ok(r.finish())
                    }
                })
            })?;
        }
        "minecraft:number_range" => {
            r.optional(
                "label_format",
                cloned,
                Some(Value::String("options.generic_value".into())),
            )?;
            r.required("start", float)?;
            r.required("end", float)?;
            r.optional("initial", float, None)?;
            r.optional("step", float, None)?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(r.finish())
}
