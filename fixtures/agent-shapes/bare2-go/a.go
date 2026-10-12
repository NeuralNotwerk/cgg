package p

type Repo struct{}

func (r *Repo) Zlist(d int) int { return d }
func (r *Repo) Caller(d int) int { return Zlist(d) }
func Top(d int) int { return Zlist(d) }
