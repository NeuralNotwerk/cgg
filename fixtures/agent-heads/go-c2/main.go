package main

import "example.com/app/internal/jobs"

func main() {
	jobs.SpawnJob(1)
	r := jobs.NewRunner()
	r.StartRunner()
}
