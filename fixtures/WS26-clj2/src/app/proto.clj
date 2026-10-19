(ns app.proto)
(defprotocol Store
  (put! [this k v] "store")
  (fetch [this k]))
