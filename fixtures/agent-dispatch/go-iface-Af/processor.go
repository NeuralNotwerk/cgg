package main

type processor interface {
	process()
	unusedOp()
}
