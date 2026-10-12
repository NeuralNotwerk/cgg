package main

func run[T processor](p T) {
	p.process()
}

func main() {
	run(alpha{})
	run(beta{})
	run(gamma{})
	run(delta{})
	run(epsilon{})
	run(zeta{})
}
