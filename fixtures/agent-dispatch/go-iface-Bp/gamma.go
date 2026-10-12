package main

import "fmt"

type gamma struct{}

func (x gamma) process() { fmt.Println("Gamma") }

func (x gamma) unusedOp() { fmt.Println("u") }
