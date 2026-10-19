(ns app.core
  (:require [app.util :as u]))

(defn- on-req [r] r)
(defn- step [x] x)
(defn- orphan [] 1)
(defn- shadowed [] 1)
(defn- quoted [] 1)

(defn -main [& args]
  (let [shadowed identity]
    (map shadowed args))
  (map step args)
  (-> args u/normalize)
  (println #'u/hook '(orphan))
  {:handler on-req})

(defn top-fn [] 2)
(register! top-fn)
