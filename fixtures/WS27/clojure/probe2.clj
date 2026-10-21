(ns p (:require [clojure.string :as str]))
(def top 1)
(defn f [x {:keys [a b] :or {a (dflt)} :as m} & more]
  (let [y (x 1) [p q] y]
    (for [i xs :let [j i] :when (j)] (i j))
    (binding [rb 1] (rb))
    (with-redefs [wr stub] (wr))
    (with-open [r (open-it fl)] (r))
    (try (go) (catch Exception e (e)))
    (letfn [(h1 [z] (h2 z)) (h2 [w] w)] (h1 1))
    (fn self [k] (self k))))
(defn g ([u] (u)) ([u v] (v u)))
(deftype T [fld] P (meth [this arg] (fld arg)))
(defmacro mac [mp] `(let [y# 1] (mp ~mp)))
