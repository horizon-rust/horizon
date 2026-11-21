//! `MessageArgument`: the `@p`-style selectors in a chat message's text, as
//! `toComponent` resolves them for a source that may use selectors.
use super::*;

/// `MessageArgument.Message.parseText` then `toComponent`: the text before
/// the first selector is the root literal, each selector is one
/// `EntitySelector.joinNames` component, and the text between them follows.
/// The bool says whether the message had a selector at all.
pub(super) fn expand_selectors(game: &mut Game<'_>, raw: &str) -> (Text, bool) {
    struct Part {
        start: usize,
        end: usize,
        names: Text,
    }
    let mut parts = Vec::new();
    let (mut offset, mut rest) = (0, raw);
    while let Some(at) = rest.find('@') {
        let start = offset + at;
        let mut reader = Reader::new(&rest[at..]);
        match (horizon_client::command::ArgType::Entity {
            single: false,
            players: false,
        })
        .parse(&mut reader, &game.ui.registries)
        {
            Ok(Arg::Entity(s)) if reader.cursor > 1 => {
                let found = game.select(&s);
                // EntitySelector.joinNames / ComponentUtils.formatList.
                let names = if found.len() == 1 {
                    game.target_name(found[0])
                } else {
                    let mut names = Text::empty();
                    for (i, t) in found.iter().enumerate() {
                        if i > 0 {
                            names.push(Text::colored(", ", text::GRAY));
                        }
                        names.push(game.target_name(*t));
                    }
                    names
                };
                offset = start + reader.cursor;
                rest = &raw[offset..];
                parts.push(Part {
                    start,
                    end: offset,
                    names,
                });
            }
            _ => {
                offset = start + 1;
                rest = &raw[offset..];
            }
        }
    }
    let Some(first) = parts.first() else {
        return (Text::literal(raw), false);
    };
    let mut out = Text::literal(&raw[..first.start]);
    let mut read_to = first.start;
    for part in &parts {
        if read_to < part.start {
            out.push(&raw[read_to..part.start]);
        }
        out.push(part.names.clone());
        read_to = part.end;
    }
    if read_to < raw.len() {
        out.push(&raw[read_to..]);
    }
    (out, true)
}
