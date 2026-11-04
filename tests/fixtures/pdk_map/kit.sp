* A made-up kit for pdk-map (tests/pdk_map.rs): device interfaces only.
.subckt nfoo d g s b w=1 l=1 nf=1 m=1
.ends nfoo
.subckt pbar d g s b w=1 l=1
.ends pbar
.subckt rbaz p n r=1 m=1
.ends rbaz
.subckt nocdf a b
.ends nocdf
.subckt qux a b
.ends qux
