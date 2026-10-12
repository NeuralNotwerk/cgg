package idv

type Provider interface{ Verify() bool }

type Real struct{}

func (r *Real) Verify() bool { return true }
