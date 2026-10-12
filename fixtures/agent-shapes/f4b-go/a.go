package p

type Repo struct{}

func (r *Repo) Quux() int { return 1 }

type Other struct{}

func (o *Other) Quux() int { return 2 }

type Svc struct{ repo *Repo }

func (s *Svc) CtlField() { s.repo.Quux() }
func (s *Svc) MultiRecv() {
	s.
		repo.
		Quux()
}
