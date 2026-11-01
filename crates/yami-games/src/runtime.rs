use crate::audio::AudioCommand;
use crate::*;
use std::sync::Arc;
use wasmi::{
    Caller, Config, Engine, Error, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
    TypedFunc,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Clear(u32),
    Sprite {
        asset: u32,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        layer: i32,
    },
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
    Circle {
        x: i32,
        y: i32,
        radius: i32,
        color: u32,
    },
    Text {
        x: i32,
        y: i32,
        size: i32,
        text: String,
        color: u32,
    },
}
#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub commands: Vec<Command>,
    pub assets: Arc<Assets>,
}
struct Host {
    limits: StoreLimits,
    commands: Vec<Command>,
    drawing: bool,
    pixels: usize,
    text_bytes: usize,
    abi: i32,
    input: Input,
    assets: Arc<Assets>,
    audio: Vec<AudioCommand>,
}
impl Host {
    fn push(&mut self, command: Command) -> Result<(), Error> {
        if !self.drawing {
            return Err(Error::new("Drawing is only allowed in update"));
        }
        if self.commands.len() >= MAX_COMMANDS {
            return Err(Error::new("Drawing command limit exceeded"));
        }
        let pixels = match &command {
            Command::Sprite { x, y, w, h, .. } => {
                let width = (x + w).min(WIDTH) - (*x).max(0);
                let height = (y + h).min(HEIGHT) - (*y).max(0);
                (width.max(0) * height.max(0)) as usize
            }
            Command::Clear(_) => (WIDTH * HEIGHT) as usize,
            Command::Rect { w, h, .. } => (w * h) as usize,
            Command::Circle { radius, .. } => (4 * radius * radius) as usize,
            Command::Text { text, .. } => {
                self.text_bytes += text.len();
                if self.text_bytes > 4096 {
                    return Err(Error::new("Frame text limit exceeded"));
                }
                0
            }
        };
        self.pixels += pixels;
        if self.pixels > 2 * (WIDTH * HEIGHT) as usize {
            return Err(Error::new("Frame drawing area limit exceeded"));
        }
        self.commands.push(command);
        Ok(())
    }
}
fn point(x: i32, y: i32) -> Result<(), Error> {
    if !(0..WIDTH).contains(&x) || !(0..HEIGHT).contains(&y) {
        return Err(Error::new("Drawing coordinate outside logical canvas"));
    }
    Ok(())
}

/// Synchronous VM for worker threads and headless validation. Never call on the UI thread.
pub struct Runtime {
    store: Store<Host>,
    input: TypedFunc<(i32, i32), ()>,
    update: TypedFunc<i32, ()>,
    reset: TypedFunc<(), ()>,
}
impl Runtime {
    pub fn new(bytes: &[u8]) -> Result<Self, String> {
        Self::with_assets(bytes, None, Arc::default())
    }
    pub fn with_assets(
        bytes: &[u8],
        expected_abi: Option<u32>,
        assets: Arc<Assets>,
    ) -> Result<Self, String> {
        if bytes.len() > MAX_MODULE_BYTES || !bytes.starts_with(b"\0asm\x01\0\0\0") {
            return Err("Expected a bounded binary WebAssembly module".into());
        }
        let mut config = Config::default();
        config.enforced_limits(wasmi::EnforcedLimits::strict());
        config.compilation_mode(wasmi::CompilationMode::Eager);
        config
            .consume_fuel(true)
            .set_max_recursion_depth(128)
            .set_max_stack_height(64 * 1024);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        let mut linker = Linker::<Host>::new(&engine);
        linker
            .func_wrap(
                "yami_game_v1",
                "clear",
                |mut c: Caller<'_, Host>, color: i32| {
                    c.data_mut().push(Command::Clear(color as u32))
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "yami_game_v1",
                "rect",
                |mut c: Caller<'_, Host>, x: i32, y: i32, w: i32, h: i32, color: i32| {
                    point(x, y)?;
                    if w <= 0 || h <= 0 || w > WIDTH - x || h > HEIGHT - y {
                        return Err(Error::new("Invalid drawing rectangle"));
                    }
                    c.data_mut().push(Command::Rect {
                        x,
                        y,
                        w,
                        h,
                        color: color as u32,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "yami_game_v1",
                "circle",
                |mut c: Caller<'_, Host>, x: i32, y: i32, radius: i32, color: i32| {
                    point(x, y)?;
                    if radius <= 0
                        || radius > x
                        || radius > y
                        || radius > WIDTH - x
                        || radius > HEIGHT - y
                    {
                        return Err(Error::new("Invalid drawing circle"));
                    }
                    c.data_mut().push(Command::Circle {
                        x,
                        y,
                        radius,
                        color: color as u32,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "yami_game_v1",
                "text",
                |mut c: Caller<'_, Host>,
                 ptr: i32,
                 len: i32,
                 x: i32,
                 y: i32,
                 size: i32,
                 color: i32| {
                    point(x, y)?;
                    if color as u32 & 255 != 255
                        || !(1..=128).contains(&len)
                        || !(8..=48).contains(&size)
                        || ptr < 0
                    {
                        return Err(Error::new("Invalid text command"));
                    }
                    let memory = c
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .ok_or_else(|| Error::new("Missing exported memory"))?;
                    let mut bytes = [0u8; 128];
                    let bytes = &mut bytes[..len as usize];
                    memory
                        .read(&c, ptr as usize, bytes)
                        .map_err(|_| Error::new("Text pointer outside memory"))?;
                    let text =
                        std::str::from_utf8(bytes).map_err(|_| Error::new("Text is not UTF-8"))?;
                    if text.chars().any(char::is_control) {
                        return Err(Error::new("Control character in game text"));
                    }
                    let command = Command::Text {
                        x,
                        y,
                        size,
                        text: text.to_owned(),
                        color: color as u32,
                    };
                    c.data_mut().push(command)
                },
            )
            .map_err(|e| e.to_string())?;
        // ABI 2 imports cannot be used by ABI 1 or during module start/negotiation.
        linker
            .func_wrap(
                "yami_game_v2",
                "input",
                |c: Caller<'_, Host>, field: i32| -> Result<i32, Error> {
                    if c.data().abi != 2 {
                        return Err(Error::new("ABI 2 capability required"));
                    }
                    let i = c.data().input;
                    match field {
                        0 => Ok(i.held as i32),
                        1 => Ok(i.pressed as i32),
                        2 => Ok(i.released as i32),
                        3 => Ok(i.x),
                        4 => Ok(i.y),
                        _ => Err(Error::new("Unknown input field")),
                    }
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "yami_game_v2",
                "sprite",
                |mut c: Caller<'_, Host>,
                 asset: i32,
                 sx: i32,
                 sy: i32,
                 sw: i32,
                 sh: i32,
                 x: i32,
                 y: i32,
                 w: i32,
                 h: i32,
                 layer: i32|
                 -> Result<(), Error> {
                    if c.data().abi != 2 {
                        return Err(Error::new("ABI 2 capability required"));
                    }
                    let texture = c
                        .data()
                        .assets
                        .texture(asset as u32)
                        .ok_or_else(|| Error::new("Unknown sprite asset"))?;
                    if sx < 0
                        || sy < 0
                        || sw <= 0
                        || sh <= 0
                        || sw > texture.width() as i32
                        || sh > texture.height() as i32
                        || sx > texture.width() as i32 - sw
                        || sy > texture.height() as i32 - sh
                        || !(-WIDTH..=WIDTH * 2).contains(&x)
                        || !(-HEIGHT..=HEIGHT * 2).contains(&y)
                        || !(1..=WIDTH * 2).contains(&w)
                        || !(1..=HEIGHT * 2).contains(&h)
                        || !(-128..=127).contains(&layer)
                    {
                        return Err(Error::new("Invalid sprite rectangle, scale or layer"));
                    }
                    c.data_mut().push(Command::Sprite {
                        asset: asset as u32,
                        sx,
                        sy,
                        sw,
                        sh,
                        x,
                        y,
                        w,
                        h,
                        layer,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "yami_game_v2",
                "audio",
                |mut c: Caller<'_, Host>,
                 op: i32,
                 channel: i32,
                 asset: i32,
                 value: i32,
                 looping: i32|
                 -> Result<(), Error> {
                    if c.data().abi != 2
                        || !c.data().drawing
                        || c.data().audio.len() >= 32
                        || !(0..8).contains(&channel)
                        || !(0..=1000).contains(&value)
                        || !(0..=1).contains(&looping)
                    {
                        return Err(Error::new("Invalid audio command or budget exceeded"));
                    }
                    let command = match op {
                        0 if c.data().assets.sound(asset as u32).is_some() => AudioCommand::Play {
                            channel: channel as u8,
                            asset: asset as u32,
                            looping: looping == 1,
                            volume: value as u16,
                        },
                        1 => AudioCommand::Stop(channel as u8),
                        2 => AudioCommand::Volume(channel as u8, value as u16),
                        3 if value <= 1 => AudioCommand::Mute(value == 1),
                        _ => return Err(Error::new("Invalid audio operation or asset")),
                    };
                    c.data_mut().audio.push(command);
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?;
        let host = Host {
            limits: StoreLimitsBuilder::new()
                .memory_size(MAX_MEMORY_BYTES)
                .memories(1)
                .tables(1)
                .table_elements(1024)
                .instances(1)
                .trap_on_grow_failure(true)
                .build(),
            commands: Vec::new(),
            drawing: false,
            pixels: 0,
            text_bytes: 0,
            abi: 0,
            input: Input {
                x: -1,
                y: -1,
                ..Input::default()
            },
            assets,
            audio: Vec::new(),
        };
        let mut store = Store::new(&engine, host);
        store.limiter(|h| &mut h.limits);
        store.set_fuel(FRAME_FUEL).map_err(|e| e.to_string())?;
        // Start functions, ABI negotiation and init share a single fuel budget.
        // Only explicitly versioned game imports exist; WASI and all other imports fail closed.
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| e.to_string())?;
        let abi = instance
            .get_typed_func::<(), i32>(&store, "yami_game_abi")
            .map_err(|e| e.to_string())?;
        let abi = abi.call(&mut store, ()).map_err(|e| e.to_string())?;
        if !(1..=ABI_VERSION).contains(&abi)
            || expected_abi.is_some_and(|expected| expected != abi as u32)
        {
            return Err("Unsupported or mismatched game ABI".into());
        }
        store.data_mut().abi = abi;
        let init = instance
            .get_typed_func::<(), ()>(&store, "yami_game_init")
            .map_err(|e| e.to_string())?;
        let input = instance
            .get_typed_func(&store, "yami_game_input")
            .map_err(|e| e.to_string())?;
        let update = instance
            .get_typed_func(&store, "yami_game_update")
            .map_err(|e| e.to_string())?;
        let reset = instance
            .get_typed_func(&store, "yami_game_reset")
            .map_err(|e| e.to_string())?;
        init.call(&mut store, ()).map_err(|e| e.to_string())?;
        Ok(Self {
            store,
            input,
            update,
            reset,
        })
    }
    pub fn reset(&mut self) -> Result<(), String> {
        self.store.data_mut().audio.clear();
        self.store.set_fuel(FRAME_FUEL).map_err(|e| e.to_string())?;
        self.reset
            .call(&mut self.store, ())
            .map_err(|e| e.to_string())
    }
    pub fn frame(&mut self, held: u32, pressed: u32) -> Result<Arc<Frame>, String> {
        self.frame_input(Input {
            held,
            pressed,
            x: -1,
            y: -1,
            ..Input::default()
        })
    }
    pub fn take_audio(&mut self) -> Vec<AudioCommand> {
        std::mem::take(&mut self.store.data_mut().audio)
    }
    pub fn frame_input(&mut self, input: Input) -> Result<Arc<Frame>, String> {
        self.store.data_mut().input = Input {
            held: input.held & INPUT_MASK,
            pressed: input.pressed & INPUT_MASK,
            released: input.released & INPUT_MASK,
            x: input.x.clamp(-1, WIDTH - 1),
            y: input.y.clamp(-1, HEIGHT - 1),
        };
        self.store.data_mut().audio.clear();
        self.store.data_mut().commands.clear();
        self.store.data_mut().pixels = 0;
        self.store.data_mut().text_bytes = 0;
        self.store.set_fuel(FRAME_FUEL).map_err(|e| e.to_string())?;
        self.input
            .call(
                &mut self.store,
                ((input.held & 15) as i32, (input.pressed & 15) as i32),
            )
            .map_err(|e| e.to_string())?;
        self.store.data_mut().drawing = true;
        let result = self.update.call(&mut self.store, TICK_US);
        self.store.data_mut().drawing = false;
        result.map_err(|e| e.to_string())?;
        let commands = std::mem::take(&mut self.store.data_mut().commands);
        if !matches!(commands.first(), Some(Command::Clear(_))) {
            return Err("Every game frame must begin with clear".into());
        }
        // Clear is always first. Other commands use stable layer order; primitives are layer 0.
        let mut commands = commands;
        commands[1..].sort_by_key(|command| match command {
            Command::Sprite { layer, .. } => *layer,
            _ => 0,
        });
        Ok(Arc::new(Frame {
            commands,
            assets: Arc::clone(&self.store.data().assets),
        }))
    }
}
