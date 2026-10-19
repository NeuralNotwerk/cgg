(ns lib.api (:require [potemkin :as p] [lib.impl]))
(p/import-vars [lib.impl exported-fn reexported-at-top])
