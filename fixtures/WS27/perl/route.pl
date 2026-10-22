package Q;
use Mojolicious::Lite;
sub handler { 1 }
sub setup { my $handler = sub { 2 }; get "/x" => $handler; }
