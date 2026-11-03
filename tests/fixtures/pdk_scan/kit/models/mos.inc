* Binned BSIM4 cards, a wrapper subckt and a protected block.
.option post=2
.model nch.1 nmos level=54 lmin=1.11e-7 lmax=9.99e-7 wmin=2.22e-7 wmax=8.88e-6
+ vth0=0.4123 toxe=1.9e-9
.model nch.2 nmos (level=54 lmin=9.99e-7 lmax=9.99e-6 wmin=2.22e-7 wmax=8.88e-6 vth0=0.4321)
.MODEL pch.1 PMOS level=54 lmin=1.11e-7 lmax=9.99e-6 vth0=-0.4567
.subckt nmos_lvt d g s b w=1u l=100n
+ nf={2 * 1}
m0 d g s b nmos_lvt_core w='w' l='l'
.model nmos_lvt_core nmos level=54 vth0=0.3789
.ends nmos_lvt
.prot
#qz8812$%encryptedbody
+ zz9081 lmin=4.44e-7
.unprot
.subckt rppoly p n l=2u w=0.5u
r0 p n rppoly_m l='l' w='w'
.ends rppoly
