//! The port a world is published on, as a copy-on-click link
//! (`ComponentUtils.copyOnClickText`): `/publish` (`PublishCommand.getSuccessMessage`)
//! and Open to LAN (`WorldOptionsScreen.publish`).
use horizon_client::text::Text;

/// `PublishCommand.getSuccessMessage`.
pub(in crate::game) fn publish_started(port: i32) -> Text {
    Text::translate(
        "commands.publish.started.lan",
        &[Text::copyable(port.to_string())],
    )
}

/// `WorldOptionsScreen.publish`, and the same message when the world is opened
/// through `lan_open`.
pub(in crate::game) fn lan_started(port: i32) -> Text {
    Text::translate(
        "menu.multiplayerOptions.publish.started.lan",
        &[Text::copyable(port.to_string())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{
        lang::{self, Language, ResourceSource},
        text::{ClickEvent, GREEN},
    };

    /// The two templates, so the test needs no prepared resources.
    struct Vanilla;
    impl ResourceSource for Vanilla {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            (path == "assets/minecraft/lang/en_us.json").then(|| {
                br#"{
                    "commands.publish.started.lan": "Local game hosted on port %s",
                    "menu.multiplayerOptions.publish.started.lan": "This world is now open to LAN. The port number is %s"
                }"#
                .to_vec()
            })
        }
    }

    /// The link a click copies and a Shift-click types, and only it: the
    /// sentence around it has no click.
    #[test]
    fn the_published_port_is_a_copy_on_click_link() {
        lang::scoped(Language::load(&[&Vanilla], "en_us"), || {
            for message in [publish_started(25565), lan_started(25565)] {
                let port = "25565";
                let link = message
                    .spans
                    .iter()
                    .find(|span| span.style.click.is_some())
                    .unwrap_or_else(|| panic!("{} has no link", message.plain()));
                assert_eq!(
                    link.style.click,
                    Some(ClickEvent::CopyToClipboard(port.into()))
                );
                assert_eq!(link.style.insertion.as_deref(), Some(port));
                assert_eq!(link.style.color, Some(GREEN));
                assert_eq!(
                    link.style.hover.as_deref().map(Text::plain),
                    Some("chat.copy.click".to_owned())
                );
                assert!(
                    message.plain().contains(&format!("[{port}]")),
                    "{}",
                    message.plain()
                );
            }
        });
    }
}
