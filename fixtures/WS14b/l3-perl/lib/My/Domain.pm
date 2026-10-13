package My::Domain;
use strict;

sub new { my $class = shift; my $self = bless {}, $class; $class->init_class(); return $self; }
sub init_class { return 1 }
sub is_public { return 1 }
sub base_only { return 2 }

sub info {
    my $self = shift;
    my %h = (
        is_public => $self->is_public,
        pkg => __PACKAGE__->init_class(),
    );
    return \%h;
}
1;
