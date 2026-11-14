//! The application that runs a `Game`'s event loop (spec #14206, step 3). With none registered
//! the game is its own handler: no compositor and no extra check per frame, as without
//! splitscreen. A platform part may register a factory that wraps the game in a handler of its
//! own (splitscreen's `Split`); the engine gets the game back when the loop ends.
use super::Game;
use std::sync::OnceLock;
use winit::application::ApplicationHandler;

/// An event-loop handler that owns a `Game` and gives it back when the loop ends.
pub(crate) trait GameApp<'a>: ApplicationHandler {
    /// The game it runs for.
    fn game(&mut self) -> &mut Game<'a>;
    fn into_game(self: Box<Self>) -> Game<'a>;
}

/// A `Game` runs as its own handler.
impl<'a> GameApp<'a> for Game<'a> {
    fn game(&mut self) -> &mut Game<'a> {
        self
    }
    fn into_game(self: Box<Self>) -> Game<'a> {
        *self
    }
}

/// Wraps a game in the handler that runs it.
pub(crate) type AppFactory = for<'a> fn(Game<'a>) -> Box<dyn GameApp<'a> + 'a>;

static FACTORY: OnceLock<AppFactory> = OnceLock::new();

/// Registers the app factory, once, before the game starts. Later ones are ignored with a warning.
pub(crate) fn register(factory: AppFactory) {
    if FACTORY.set(factory).is_err() {
        log::warn!("An app factory is already registered; ignoring the second");
    }
}

/// The handler that runs `game`: the registered factory's, else the game itself.
pub(super) fn app_for<'a>(game: Game<'a>) -> Box<dyn GameApp<'a> + 'a> {
    match FACTORY.get() {
        Some(factory) => factory(game),
        None => Box::new(game),
    }
}
