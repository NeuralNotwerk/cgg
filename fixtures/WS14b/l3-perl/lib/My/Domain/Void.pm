package My::Domain::Void;
use parent -norequire, 'My::Domain';

sub is_active { return 0 }

sub status {
    my $self = shift;
    return ( active => $self->is_active, b => $self->base_only, pub => $self->is_public );
}
1;
