(ns app.core (:require [app.proto :as p] [lib.api :as api] [app.util :as u]))
(deftype Mem [m]
  p/Store
  (fetch [this k] (get m k))
  (unused-op [this] nil)
  Runnable
  (run [this] (u/from-run)))
(defn handler [st] (p/fetch st :a) (api/exported-fn 1))
(u/at-top)
(api/reexported-at-top)
