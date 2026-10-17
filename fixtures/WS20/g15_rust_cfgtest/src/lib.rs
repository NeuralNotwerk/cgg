pub fn used() -> u32 { 1 }

fn prod_dead() -> u32 { 2 }

#[cfg(test)]
fn cfg_test_helper() -> u32 { 3 }

#[cfg(test)]
mod tests {
    use super::*;

    fn unused_test_helper() -> u32 { 4 }

    fn used_test_helper() -> u32 { used() }

    #[test]
    fn it_works() { assert_eq!(used_test_helper(), 1); }

    mod nested {
        fn nested_helper() {}
    }
}

#[cfg(all(test, feature = "x"))]
mod more_tests {
    fn helper2() {}
}

#[cfg(not(test))]
fn not_test_dead() {}
