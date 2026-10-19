(ns h.core (:require [h.sql :as sql] [h.refd :refer [picked]] [h.whole :refer :all]))
(defn run [a b t] (- a b) (helper a) (sql/helper a) (picked a) (not-picked a) (every a) (get t a))
