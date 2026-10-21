(ns app.core)

;; Functions whose names the bindings below reuse.
(defn cb [x] x)
(defn handler [x] x)
(defn item [x] x)
(defn step [x] x)
(defn err [x] x)
(defn helper [x] x)
(defn later [x] x)
(defn stub [x] x)
(defn now [] 0)

;; A parameter, a destructured key, a `let`, a `for`, a `catch`, a `letfn`.
(defn with-param [cb] (cb 1))
(defn with-keys [{:keys [handler]}] (handler 2))
(defn with-let [] (let [item (now)] (item 3)))
(defn with-for [fs] (for [step fs] (step 4)))
(defn with-catch [] (try (now) (catch Exception err (err 5))))
(defn with-letfn [] (letfn [(helper [x] x)] (helper 6)))

;; Not locals: `binding` rebinds the var; the init sees the function; a
;; qualified call; generated code in a syntax-quote.
;; (`later` is rebound below; `binding` does not make it a local.)
(defn with-binding [] (binding [later inc] (later 7)))
(defn with-init [] (let [now (now)] now))
(defn with-qualified [stub] (app.core/stub stub))
(defmacro with-template [cb] `(cb ~cb))
