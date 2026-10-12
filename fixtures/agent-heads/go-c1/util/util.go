package util

type Errs struct{}

func (e *Errs) Wrap(err error, msg string) error { return err }
func (e *Errs) Wrapf(err error, msg string) error { return err }

type IDs struct{}

func (i IDs) NewString() string { return "" }
func (i IDs) MustParse(s string) string { return s }

func Infow(msg string) {}
