//! Windowed start-up: imports the game data in a background thread while a
//! simple progress screen is shown, then hands over to the real scene.

use std::rc::Rc;
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::Result;
use glam::Vec2;
use rounds_import::GameData;
use rounds_render::{Color, Layer};

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::data::{self, Plan};

type MakeScene = Box<dyn FnOnce(Rc<GameData>) -> Result<Box<dyn Scene>>>;

enum State {
    Working(JoinHandle<Result<GameData>>),
    Ready(Box<dyn Scene>),
    Failed(String),
    Empty,
}

/// Runs [`data::run_import`] + [`data::load`] off the main thread, then
/// creates the scene with `make`.
pub struct BootScene {
    plan: Plan,
    make: Option<MakeScene>,
    state: State,
    started: Instant,
}

impl BootScene {
    pub fn new(plan: Plan, make: MakeScene) -> Self {
        BootScene {
            plan,
            make: Some(make),
            state: State::Empty,
            started: Instant::now(),
        }
    }
}

impl Scene for BootScene {
    fn load(&mut self, _ctx: &mut LoadContext<'_>) -> Result<()> {
        let plan = self.plan.clone();
        self.started = Instant::now();
        self.state = State::Working(std::thread::spawn(move || {
            data::run_import(&plan)?;
            data::load(&plan.import_dir)
        }));
        Ok(())
    }

    fn poll(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        match &mut self.state {
            State::Working(h) if h.is_finished() => {
                let State::Working(h) = std::mem::replace(&mut self.state, State::Empty) else {
                    unreachable!()
                };
                let result = h
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("import thread panicked")));
                self.state = match result.and_then(|d| (self.make.take().unwrap())(Rc::new(d))) {
                    Ok(mut scene) => match scene.load(ctx) {
                        Ok(()) => State::Ready(scene),
                        Err(e) => State::Failed(format!("{e:#}")),
                    },
                    Err(e) => State::Failed(format!("{e:#}")),
                };
                if let State::Failed(msg) = &self.state {
                    log::error!("{msg}");
                }
                Ok(())
            }
            State::Ready(s) => s.poll(ctx),
            _ => Ok(()),
        }
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        if let State::Ready(s) = &mut self.state {
            s.tick(ctx);
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        if let State::Ready(s) = &mut self.state {
            return s.draw(ctx);
        }
        let (w, h) = (ctx.size.0 as f32, ctx.size.1 as f32);
        ctx.frame.clear = Color::hex(0x101010);
        let px = (h / 24.0).max(14.0);
        let centre = Vec2::new(w * 0.5, h * 0.45);
        let (title, detail) = match &self.state {
            State::Failed(msg) => ("Could not load the game data".to_string(), msg.clone()),
            _ => {
                let dots = ".".repeat((self.started.elapsed().as_secs_f32() * 2.0) as usize % 4);
                let from = self
                    .plan
                    .install
                    .as_ref()
                    .map_or(String::new(), |i| format!("from {}", i.dir.display()));
                (
                    format!("Importing ROUNDS data{dots}"),
                    format!(
                        "{from}\ninto {}\n(first start or after a game update only)",
                        self.plan.import_dir.display()
                    ),
                )
            }
        };
        ctx.list.text(
            Layer::Screen,
            0,
            title,
            centre,
            px,
            Vec2::new(0.5, 1.0),
            Color::rgb(0.902, 0.902, 0.902),
        );
        ctx.list.text(
            Layer::Screen,
            0,
            detail,
            centre + Vec2::new(0.0, px),
            px * 0.55,
            Vec2::new(0.5, 0.0),
            Color::rgb(0.553, 0.553, 0.553),
        );
    }
}
