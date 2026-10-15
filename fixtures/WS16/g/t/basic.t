use strict;
use App::Group;
sub user_admin { return App::Group->new(); }
sub name { return "x"; }
sub uid { return 3; }
my %h = (
    uid => user_admin->id,
    name => 'fixed',
);
my $r = { uid => 1, name => 2 };
print uid();
