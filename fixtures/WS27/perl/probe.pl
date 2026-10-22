package P;
sub f { 1 }
sub g {
    my ($cb, $x) = @_;
    my $h = shift;
    $cb->(1);
    &$h(2);
    &{$cb}(3);
    f(4);
    &f(5);
    $x->$h();
    foreach my $it (@list) { $it->(6) }
    my $anon = sub { my ($p) = @_; $p->(7) };
}
sub s2 ($a, $b = 1) { $a->(8) }
