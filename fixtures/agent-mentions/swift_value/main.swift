func mapTarget(_ x: Int) -> Int { return x + 1 }
private func privMapTarget(_ x: Int) -> Int { return x }
func deadControl() {}
private func privDeadControl() {}
func calledControl() {}
let ys = [1, 2].map(mapTarget)
let zs = [1, 2].map(privMapTarget)
calledControl()
