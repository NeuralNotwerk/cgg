(ns app.proto)
(defprotocol Store (fetch [this k]) (unused-op [this]))
