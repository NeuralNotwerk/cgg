(ns app.core
  (:require [app.util :as u]))

(defn helper [x] x)

(defn outer [xs]
  (letfn [(even2? [n] (if (zero? n) true (odd2? (dec n))))
          (odd2? [n] (if (zero? n) false (even2? (dec n))))
          (multi ([a] (multi a 1)) ([a b] (+ a b)))]
    (map multi xs)
    (helper (even2? 4))))

(defn other [] (even2? 3))
