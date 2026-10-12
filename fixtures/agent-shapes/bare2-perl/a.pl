package Repo;
sub new { bless {}, shift }
sub zlist { my ($self, $d) = @_; return $d; }
package main;
sub top { my $data = shift; return zlist($data); }
