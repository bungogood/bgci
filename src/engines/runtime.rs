use bkgm::Game;
use bkgm::dice::Dice;
use bkgm::ubgi::{UbgiEngine, run_stdio_loop};

pub trait UbgiAdapter {
    fn id_name(&self) -> &'static str;
    fn id_version(&self) -> &'static str;
    fn on_ready(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<String, String>;
}

struct AdapterBridge<'a, T> {
    inner: &'a mut T,
}

impl<T: UbgiAdapter> UbgiEngine for AdapterBridge<'_, T> {
    fn id_name(&self) -> &'static str {
        self.inner.id_name()
    }

    fn id_version(&self) -> &'static str {
        self.inner.id_version()
    }

    fn id_author(&self) -> &'static str {
        "bgci"
    }

    fn on_ready(&mut self) -> Result<(), String> {
        self.inner.on_ready()
    }

    fn choose_move(&mut self, game: &Game, dice: Dice) -> Result<String, String> {
        self.inner.choose_move(game, dice)
    }
}

pub fn run_ubgi_loop(adapter: &mut impl UbgiAdapter) {
    let mut bridge = AdapterBridge { inner: adapter };
    run_stdio_loop(&mut bridge);
}
