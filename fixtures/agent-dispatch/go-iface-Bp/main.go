package main

func run(p processor) {
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
