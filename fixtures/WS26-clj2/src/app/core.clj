(ns app.core
  (:require [app.proto :as p]
            [lib.malli :as mu]
            [schema.core :as s]))

#_(defn commented [] (gone))
(defn- helper [x] x)
(defn- used-by-method [x] x)
(mu/defn typed :- :int [x :- :int] (helper x))
(s/defn checked :- s/Int [x :- s/Int] x)
(mu/other not-a-def [x] x)

(deftype MemStore [m]
  p/Store
  (put! [this k v] (used-by-method v))
  (fetch [this k] (get m k)))

(defn run [st] (p/fetch st :a) (typed 1) (checked 2))
