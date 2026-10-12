package main

type holder struct {
	field processor
}

func (h holder) runField() {
	h.field.process()
}

func main() {
	holder{field: alpha{}}.runField()
	holder{field: beta{}}.runField()
}
