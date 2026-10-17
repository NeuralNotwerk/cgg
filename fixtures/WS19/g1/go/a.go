package a

type Service struct{}

func (s *Service) Run() {}

func main() {
	service := mk()
	service.Run()
}
