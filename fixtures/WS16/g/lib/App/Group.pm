package App::Group;
sub new { my $class = shift; return bless {}, $class; }
sub id { my $self = shift; return $self->{id}; }
sub name { my $self = shift; return $self->{name}; }
1;
