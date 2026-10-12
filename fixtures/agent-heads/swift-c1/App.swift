import Foundation
import Alamofire
func run() {
    let d = try? JSONDecoder().decode(Int.self, from: Data())
    AF.request("u")
    _ = d
}
