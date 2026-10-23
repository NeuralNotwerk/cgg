(ns app.util)
(defn even2? [n] (= 0 (mod n 2)))
(defn user [] (even2? 2))
