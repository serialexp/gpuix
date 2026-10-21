use std::sync::{Arc, Mutex};
use std::time::Instant;

use gpuix_native::retained_tree::RetainedTree;

const SOLID: &str = include_str!("../../lua/gpuix/solid.lua");
const HOST: &str = include_str!("../../lua/gpuix/host.lua");
const WORKLOAD: &str = include_str!("mahina/workload.lua");

#[derive(Default)]
struct Host {
    tree: RetainedTree,
    next_id: u64,
    text_updates: usize,
}

impl Host {
    fn create(&mut self, kind: String) -> i64 {
        self.next_id += 1;
        self.tree.create_element(self.next_id, kind);
        self.next_id as i64
    }

    fn text(&mut self, id: i64, value: String) {
        self.text_updates += 1;
        self.tree.set_text(id as u64, value);
    }

    fn children(&mut self, parent: i64, children: Vec<i64>) {
        self.tree.replace_children(
            parent as u64,
            children.into_iter().map(|id| id as u64).collect(),
        );
    }

    fn destroy(&mut self, id: i64) {
        self.tree.destroy_element(id as u64);
    }

    fn root(&mut self, id: i64) {
        self.tree.set_root(id as u64);
    }
}

enum Engine {
    Mahina(mahina::Vm),
    Lua(mlua::Lua),
}

impl Engine {
    fn new(name: &str, host: Arc<Mutex<Host>>) -> anyhow::Result<Self> {
        match name {
            "mahina" | "mahina-no-jit" => {
                let vm = mahina::Vm::with_config(mahina::Config {
                    jit: Some(name == "mahina"),
                })?;
                macro_rules! register {
                    ($name:literal, $method:ident, $($argument:ident : $convert:ident),*) => {{
                        let host = host.clone();
                        vm.register_fn($name, move |_, args| {
                            let mut arguments = args.iter();
                            $(let $argument = arguments.next()
                                .ok_or("missing host argument")?.$convert()
                                .map_err(|error| error.to_string())?;)*
                            let result = host.lock().unwrap().$method($($argument),*);
                            Ok(result_values(result))
                        })?;
                    }};
                }
                register!("__gpx_create", create, kind: string_value);
                register!("__gpx_text", text, id: integer_value, text: string_value);
                let children_host = host.clone();
                vm.register_fn("__gpx_children", move |_, args| {
                    let parent = args.first().ok_or("missing parent")?.integer_value()?;
                    let table = args
                        .get(1)
                        .ok_or("missing children")?
                        .as_table()
                        .map_err(|error| error.to_string())?;
                    let length = table.len().map_err(|error| error.to_string())?;
                    let mut children = Vec::with_capacity(length as usize);
                    for index in 1..=length {
                        children.push(
                            table
                                .get(index)
                                .map_err(|error| error.to_string())?
                                .integer_value()?,
                        );
                    }
                    children_host.lock().unwrap().children(parent, children);
                    Ok(Vec::new())
                })?;
                register!("__gpx_destroy", destroy, id: integer_value);
                register!("__gpx_root", root, id: integer_value);
                Ok(Self::Mahina(vm))
            }
            "lua" => {
                let lua = mlua::Lua::new();
                macro_rules! register {
                    ($name:literal, $method:ident, $arguments:pat_param => $types:ty, $($argument:ident),*) => {{
                        let host = host.clone();
                        let function = lua.create_function(move |_, $arguments: $types| {
                            Ok(host.lock().unwrap().$method($($argument),*))
                        })?;
                        lua.globals().set($name, function)?;
                    }};
                }
                register!("__gpx_create", create, kind => String, kind);
                register!("__gpx_text", text, (id, text) => (i64, String), id, text);
                register!("__gpx_children", children, (parent, children) => (i64, Vec<i64>), parent, children);
                register!("__gpx_destroy", destroy, id => i64, id);
                register!("__gpx_root", root, id => i64, id);
                Ok(Self::Lua(lua))
            }
            _ => anyhow::bail!("engine must be mahina, mahina-no-jit, or lua"),
        }
    }

    fn eval(&self, source: &str) -> anyhow::Result<()> {
        match self {
            Self::Mahina(vm) => {
                vm.eval(source, "gpuix-experiment")?;
            }
            Self::Lua(lua) => lua.load(source).exec()?,
        }
        Ok(())
    }

    fn function(&self, name: &str) -> anyhow::Result<Function> {
        Ok(match self {
            Self::Mahina(vm) => Function::Mahina(vm.global(name)?.as_function()?.clone()),
            Self::Lua(lua) => Function::Lua(lua.globals().get(name)?),
        })
    }
}

trait Scalar {
    fn integer_value(&self) -> Result<i64, String>;
    fn string_value(&self) -> Result<String, String>;
}

impl Scalar for mahina::Value {
    fn integer_value(&self) -> Result<i64, String> {
        self.as_int().map_err(|error| error.to_string())
    }

    fn string_value(&self) -> Result<String, String> {
        let bytes = self.as_str().map_err(|error| error.to_string())?;
        String::from_utf8(bytes.as_bytes().to_vec()).map_err(|error| error.to_string())
    }
}

trait Results {
    fn values(self) -> Vec<mahina::Value>;
}

impl Results for i64 {
    fn values(self) -> Vec<mahina::Value> {
        vec![mahina::Value::Int(self)]
    }
}

impl Results for () {
    fn values(self) -> Vec<mahina::Value> {
        Vec::new()
    }
}

fn result_values(result: impl Results) -> Vec<mahina::Value> {
    result.values()
}

enum Function {
    Mahina(mahina::Function),
    Lua(mlua::Function),
}

impl Function {
    fn call(&self, argument: i64) -> anyhow::Result<()> {
        match self {
            Self::Mahina(function) => function.call::<_, ()>(argument)?,
            Self::Lua(function) => function.call::<()>(argument)?,
        }
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let name = arguments.next().unwrap_or_else(|| "mahina".into());
    let rows: i64 = arguments.next().unwrap_or_else(|| "1000".into()).parse()?;
    let warmup: i64 = std::env::var("WARMUP")
        .unwrap_or_else(|_| "200".into())
        .parse()?;
    anyhow::ensure!(
        (1..=1_000_000).contains(&warmup),
        "WARMUP must be between 1 and 1000000"
    );
    anyhow::ensure!(
        rows >= 2 && rows <= 100_000,
        "rows must be between 2 and 100000"
    );
    let host = Arc::new(Mutex::new(Host::default()));
    let start = Instant::now();
    let engine = Engine::new(&name, host.clone())?;
    engine.eval(&format!(
        "package.preload['gpuix.host'] = function() {HOST} end\n\
         package.preload['gpuix.solid'] = function() {SOLID} end\n{WORKLOAD}"
    ))?;
    let setup = start.elapsed();
    let mount = engine.function("mount")?;
    let update = engine.function("update")?;
    let reverse = engine.function("reverse")?;
    let unmount = engine.function("unmount")?;
    let start = Instant::now();
    mount.call(rows)?;
    let mount_time = start.elapsed();
    anyhow::ensure!(host.lock().unwrap().tree.elements.len() == rows as usize * 5 + 5);
    engine.function("verify")?.call(rows)?;
    for index in 0..warmup {
        update.call(index % rows + 1)?;
    }
    let mut updates = Vec::new();
    for index in 0..30 {
        let before = host.lock().unwrap().text_updates;
        let start = Instant::now();
        update.call((index + warmup) % rows + 1)?;
        updates.push(start.elapsed().as_secs_f64() * 1000.0);
        anyhow::ensure!(
            host.lock().unwrap().text_updates - before == 2,
            "only two row labels should change"
        );
    }
    let mut reorders = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        reverse.call(0)?;
        reorders.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let nodes = host.lock().unwrap().tree.elements.len();
    anyhow::ensure!(
        host.lock()
            .unwrap()
            .tree
            .elements
            .values()
            .filter(|node| node.content.as_deref() == Some("selected"))
            .count()
            == 1
    );
    engine.function("trim")?.call(0)?;
    anyhow::ensure!(
        host.lock().unwrap().tree.elements.len() == 10,
        "removed rows leaked host nodes"
    );
    unmount.call(0)?;
    anyhow::ensure!(
        host.lock().unwrap().tree.elements.is_empty(),
        "unmount leaked host nodes"
    );
    updates.sort_by(f64::total_cmp);
    reorders.sort_by(f64::total_cmp);
    let runtime = match name.as_str() {
        "lua" if cfg!(feature = "luajit") => "luajit",
        "lua" => "lua54",
        "mahina-no-jit" => "bytecode",
        _ if cfg!(all(target_os = "linux", target_arch = "x86_64")) => "native-enabled",
        _ => "trace-ir",
    };
    println!("engine={name} runtime={runtime} rows={rows} nodes={nodes} warmup={warmup} setup_ms={:.3} mount_ms={:.3} update_median_ms={:.3} update_max_ms={:.3} reverse_median_ms={:.3}",
        setup.as_secs_f64() * 1000.0, mount_time.as_secs_f64() * 1000.0,
        updates[updates.len() / 2], updates[updates.len() - 1], reorders[reorders.len() / 2]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lifecycle(name: &str) -> anyhow::Result<()> {
        let host = Arc::new(Mutex::new(Host::default()));
        let engine = Engine::new(name, host.clone())?;
        engine.eval(&format!(
            "package.preload['gpuix.host'] = function() {HOST} end\n\
             package.preload['gpuix.solid'] = function() {SOLID} end\n{WORKLOAD}"
        ))?;
        engine.function("mount")?.call(4)?;
        assert_eq!(host.lock().unwrap().tree.elements.len(), 25);
        let row_ids = || {
            let host = host.lock().unwrap();
            let root = &host.tree.elements[&host.tree.root_id.unwrap()];
            let content = &host.tree.elements[&root.children[1]];
            host.tree.elements[&content.children[0]].children.clone()
        };
        let original = row_ids();
        engine.function("reverse")?.call(0)?;
        assert_eq!(
            row_ids(),
            original.iter().rev().copied().collect::<Vec<_>>()
        );
        engine.function("reverse")?.call(0)?;
        assert_eq!(row_ids(), original);
        engine.function("verify")?.call(4)?;
        let before = host.lock().unwrap().text_updates;
        engine.function("update")?.call(2)?;
        assert_eq!(host.lock().unwrap().text_updates - before, 2);
        engine.function("update")?.call(2)?;
        assert_eq!(host.lock().unwrap().text_updates - before, 2);
        engine.function("trim")?.call(0)?;
        assert_eq!(host.lock().unwrap().tree.elements.len(), 10);
        assert_eq!(row_ids(), vec![original[0]]);
        engine.function("unmount")?.call(0)?;
        assert!(host.lock().unwrap().tree.elements.is_empty());
        assert!(host.lock().unwrap().tree.root_id.is_none());
        engine.eval("assert(not pcall(__gpx_text)); assert(1 + 2 == 3)")?;
        engine.function("mount")?.call(2)?;
        assert_eq!(host.lock().unwrap().tree.elements.len(), 15);
        engine.function("unmount")?.call(0)?;
        assert!(host.lock().unwrap().tree.elements.is_empty());
        Ok(())
    }

    #[test]
    fn mahina_lifecycle() -> anyhow::Result<()> {
        lifecycle("mahina")
    }

    #[test]
    fn mahina_interpreter_lifecycle() -> anyhow::Result<()> {
        lifecycle("mahina-no-jit")
    }

    #[test]
    fn lua_lifecycle() -> anyhow::Result<()> {
        lifecycle("lua")
    }
}
