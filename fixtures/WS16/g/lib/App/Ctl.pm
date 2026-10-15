package App::Ctl;
sub releases { return [] }
sub release { return 1 }
sub show {
    my ( $self, $c, @path ) = @_;
    my $releases = {};
    $c->stash( {
        total    => $releases->{total} // scalar @{ $releases->{releases} },
        releases => $releases->{releases},
    } );
    $c->stash( {
        files     => 1,
        release   => shift @path,
    } );
}
1;
