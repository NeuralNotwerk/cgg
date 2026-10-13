use rocket::fairing::{Fairing, Info};

struct Cors;

impl Fairing for Cors {
    fn info(&self) -> Info {
        Info::default()
    }
}

trait Store {
    fn load(&self) -> u32;
}

struct Mem;

impl Store for Mem {
    fn load(&self) -> u32 {
        1
    }
}

struct Lonely;

impl Lonely {
    fn never(&self) -> u32 {
        2
    }
}

fn use_store(s: &dyn Store) -> u32 {
    s.load()
}

fn main() {
    let _ = use_store(&Mem);
    let _ = Cors;
}
