(ns p)
(defn f [x] (letfn [(helper [y] (helper y))] (helper x)))
(defmacro m [g] `(let [y# 1] (g ~g)))
