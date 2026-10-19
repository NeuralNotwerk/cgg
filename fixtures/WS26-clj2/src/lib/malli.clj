(ns lib.malli)
(defmacro defn [& args] `(clojure.core/defn ~@args))
(defmacro other [& args] nil)
