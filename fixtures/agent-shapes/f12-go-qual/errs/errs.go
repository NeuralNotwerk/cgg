package errs

func IsA[T error](err error) bool { return err != nil }
func IsB[T error](err error) bool { return err != nil }
func Plain(err error) bool { return err != nil }
