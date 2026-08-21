* ngspice deck: .control block, ; inline comments, node names containing ';'
* and meas-result dereferences ($&) — all must survive a round-trip.
.options reltol = 1e-4
.control
run
plot v(out)
.endc

R1 net;1 0 1k ; series element
R2 net;1 out 2k
B1 out 0 V = a-$&b
.meas tran ir AVG v(out) FROM $&t1 TO = $&t2
.end
