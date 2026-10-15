use ntex::web;
async fn index() -> &'static str { "" }
async fn other() -> &'static str { "" }
fn dead() {}
#[ntex::main]
async fn main() {
    web::HttpServer::new(|| web::App::new().service(web::resource("/").to(index)).route("/o", web::get().to(other)));
}
