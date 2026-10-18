class Box {
  Box([int]$x) { }
}
function Make {
  $b = [Box]::new(1)
  $c = New-Object Box 1
}
