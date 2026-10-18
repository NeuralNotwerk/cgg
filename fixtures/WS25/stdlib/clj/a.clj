(ns a.core (:require [clojure.string :as str]))
(defn f [xs] (clojure.string/join "," xs))
(defn g [xs] (str/join "," xs))
