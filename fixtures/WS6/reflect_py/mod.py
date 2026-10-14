class Worker:
    def _handle_ping(self):
        pass

    def _handle_unreferenced(self):
        pass

    def dispatch(self, kind):
        fn = getattr(self, "_handle_ping")
        return fn()


if __name__ == "__main__":
    Worker().dispatch("x")
