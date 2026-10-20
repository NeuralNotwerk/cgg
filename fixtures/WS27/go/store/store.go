package store

// Getter declares the method; Store embeds it.
type Getter interface {
	Get() string
}

// Store is what a parameter named like this package holds.
type Store interface {
	Getter
	Put(string)
}
