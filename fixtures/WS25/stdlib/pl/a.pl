use POSIX;
use File::Spec;
sub f { my $x = POSIX::floor(1.5); my $p = File::Spec->catfile("a", "b"); }
