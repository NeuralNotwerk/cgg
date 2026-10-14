package errs

func IsA[E error](err error) bool { return false }
func As2[E error, F error](err error, x int) bool { return false }
