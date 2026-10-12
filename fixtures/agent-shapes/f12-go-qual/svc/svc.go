package svc

import "example.com/m/errs"

type NotFound struct{}

func (NotFound) Error() string { return "nf" }

func UseGen(err error) bool { return errs.IsA[*NotFound](err) }
func UseGenStmt(err error) { errs.IsB[NotFound](err) }
func UseCtl(err error) bool { return errs.Plain(err) }
