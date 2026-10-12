package Repo;
sub new { bless {}, shift }
sub zlist { my ($self, $d) = @_; return $d; }
sub use_own { my $d = shift; return zlist($d); }
package main;
sub zlist2 { return 1; }
sub top { my $data = shift; zlist2(); return zlist($data); }
