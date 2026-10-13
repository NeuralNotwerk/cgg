package My::Other;
our @ISA = ('My::Domain');
sub is_public { return 3 }
sub is_active { return 4 }
sub base_only { return 5 }
sub run { my $self = shift; return $self->is_public; }
sub nothing { my $self = shift; return $self->unknown_m; }
1;
