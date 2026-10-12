(ns app.gen)
(defmacro defgetter [n]
  `(defn ~(symbol (str "get-" n)) [] ~(str n)))
(defmacro defhandler [n & body]
  `(defn ~n [] ~@body))
