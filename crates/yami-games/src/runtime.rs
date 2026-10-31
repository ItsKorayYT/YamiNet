use crate::*;
use std::sync::Arc;
use wasmi::{
    Caller, Config, Engine, Error, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
    TypedFunc,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Clear(u32),
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
}
struct Host {
    limits: StoreLimits,
    commands: Vec<Command>,
    drawing: bool,
    pixels: usize,
    text_bytes: usize,
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
        };
        let mut store = Store::new(&engine, host);
        store.limiter(|h| &mut h.limits);
        store.set_fuel(FRAME_FUEL).map_err(|e| e.to_string())?;
        // Start functions, ABI negotiation and init share a single fuel budget.
        // Only the four drawing imports above exist; WASI and all other imports fail closed.
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| e.to_string())?;
        let abi = instance
            .get_typed_func::<(), i32>(&store, "yami_game_abi")
            .map_err(|e| e.to_string())?;
        if abi.call(&mut store, ()).map_err(|e| e.to_string())? != ABI_VERSION {
            return Err("Unsupported game ABI".into());
        }
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
        self.store.set_fuel(FRAME_FUEL).map_err(|e| e.to_string())?;
        self.reset
            .call(&mut self.store, ())
            .map_err(|e| e.to_string())
    }
    pub fn frame(&mut self, held: u32, pressed: u32) -> Result<Arc<Frame>, String> {
        self.store.data_mut().commands.clear();
        self.store.data_mut().pixels = 0;
        self.store.data_mut().text_bytes = 0;
        self.store.set_fuel(FRAME_FUEL).map_err(|e| e.to_string())?;
        self.input
            .call(&mut self.store, ((held & 15) as i32, (pressed & 15) as i32))
            .map_err(|e| e.to_string())?;
        self.store.data_mut().drawing = true;
        let result = self.update.call(&mut self.store, TICK_US);
        self.store.data_mut().drawing = false;
        result.map_err(|e| e.to_string())?;
        let commands = std::mem::take(&mut self.store.data_mut().commands);
        if !matches!(commands.first(), Some(Command::Clear(_))) {
            return Err("Every game frame must begin with clear".into());
        }
        Ok(Arc::new(Frame { commands }))
    }
}
