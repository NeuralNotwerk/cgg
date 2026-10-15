use salvo::prelude::*;
use async_graphql::Object;
use pyo3::prelude::*;
use tauri;
async fn get_all_users() {}
async fn create_user() {}
fn helper_unused() {}
fn main() {
    let r = Router::with_path("").get(get_all_users).post(create_user);
    serve(r);
}
#[tauri::command]
fn perf_state() {}
pub struct QueryRoot;
#[Object]
impl QueryRoot {
    async fn hero(&self) -> u32 { 1 }
}
#[pyclass]
struct It;
#[pymethods]
impl It {
    fn __next__(&mut self) -> Option<u32> { None }
}
impl It {
    fn not_exported(&self) {}
}
#[defmt::panic_handler]
fn panic() -> ! { loop {} }
#[cfg(test)]
mod tests {
    #[ntex::test]
    async fn t_ntex() {}
    #[crate::rt_test]
    async fn t_rt() {}
    #[wasm_bindgen_test]
    fn t_wasm() {}
    fn helper_test_data() {}
}
