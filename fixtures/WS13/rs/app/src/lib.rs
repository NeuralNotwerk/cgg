use extweb::web::Data;

pub struct Ctx {
    n: u32,
}

impl Ctx {
    pub fn pool(&self) -> u32 {
        self.n
    }
}

// `Data` here is the external crate's wrapper: it dereferences to `Ctx`.
pub fn handler(context: Data<Ctx>) -> u32 {
    context.pool()
}

pub struct Loader {
    path: String,
}

pub struct Config {
    v: u32,
}

impl Loader {
    pub fn new(path: String) -> Loader {
        Loader { path }
    }
    // An associated fn that does NOT return Self.
    pub fn load(path: &str) -> Config {
        Config { v: path.len() as u32 }
    }
    pub fn value(&self) -> u32 {
        self.path.len() as u32
    }
}

impl Config {
    pub fn value(&self) -> u32 {
        self.v
    }
}

pub fn use_loader() -> u32 {
    let cfg = Loader::load("x");
    let l = Loader::new(String::new());
    cfg.value() + l.value()
}

macro_rules! make_getter {
    ($name:ident) => {
        pub fn $name() -> u32 {
            7
        }
    };
}

make_getter!(answer);

fn has_source(x: u32) -> u32 {
    x
}

fn no_source(_x: u32) -> u32 {
    0
}

macro_rules! table {
    ( $( $name:ident : $f:expr ),+ $(,)? ) => {
        pub fn dispatch(k: u32) -> u32 {
            match k { $( _ if stringify!($name).len() as u32 == k => $f(k), )+ _ => 0 }
        }
    };
}

table! {
    one: has_source,
    two: no_source,
}

pub fn caller() -> u32 {
    answer()
}
