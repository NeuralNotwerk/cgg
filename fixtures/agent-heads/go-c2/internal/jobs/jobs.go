package jobs

func SpawnJob(x int) int { return x }

type Runner struct{}

func (r *Runner) StartRunner() int { return 1 }
func NewRunner() *Runner { return &Runner{} }
