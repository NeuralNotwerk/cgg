import AWSLambdaRuntime

Lambda.run { (context, name: String, callback: @escaping (Result<String, Error>) -> Void) in
    callback(.success(greet(name)))
}

func greet(_ n: String) -> String { return "hi " + n }

func notAHandler() {}

// Near miss: `run` with another receiver mints no handler.
func work() async {
    await MainActor.run {
        notAHandlerEither()
    }
}
func notAHandlerEither() {}
