(ns app.core
  (:require [app.gen :refer [defgetter defhandler]]))
(defgetter color)
(defhandler handle-thing (println "x"))
(defn real-fn [] 1)
(defn use-it []
  (get-color)
  (handle-thing)
  (real-fn))
