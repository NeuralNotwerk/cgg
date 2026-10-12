package ttl

type Duration int

var CacheDefault Duration = 1

func (d Duration) Int() int { return int(d) }

func Seconds() int { return 1 }
