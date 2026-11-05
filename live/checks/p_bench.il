;; p_bench.il: lane P live check (gaps2 P15), run by p_bench.sh after it exported a Schemify
;; bench over the kit inverter with foundry corners ($SFYCHECK_PSECTIONS) to out/p15.sfyx and
;; its OCEAN script to out/p15_ocean. Loads it into chkLib (SfyImport), then per corner
;; netlists the maestro test with ADE (maeCreateNetlistForCorner) and runs Spectre on it:
;; the netlist must include the kit's models at that section and Spectre must finish; then
;; the OCEAN script runs in an `ocean -nograph` of its own and prints the output per corner.
chkW = getShellEnvVar("SFYCHECK_WORK")
chkOut = strcat(chkW "/out/p15_live")
chkSim = strcat(chkW "/sim_p")
system(sprintf(nil "rm -rf '%s' '%s'; mkdir -p '%s'" chkOut chkSim chkOut))
envSetVal("asimenv.startup" "projectDir" 'string chkSim)

procedure(chkPLines(path)
  let((port l out)
    port = and(isFile(path) infile(path))
    when(port
      while(gets(l port)
        out = cons(if(rexMatchp("\n$" l) substring(l 1 strlen(l) - 1) l) out))
      close(port))
    reverse(out)))

let((ok sess test d nl sp corners)
  corners = parseString(or(getShellEnvVar("SFYCHECK_PSECTIONS") "") " ")
  ok = chkE("SfyImport p15.sfyx" lambda(() SfyImport(strcat(chkW "/out/p15.sfyx") ?lib chkLib)))
  chkThat("SfyImport of the exported bench" ok)
  sess = chkE("open maestro" lambda(() maeOpenSetup(chkLib "inv_bench" "maestro" ?mode "a")))
  test = car(chkE("tests" lambda(() maeGetSetup(?session sess))))
  foreach(c corners
    d = strcat(chkOut "/" c)
    chkE(sprintf(nil "netlist %s" c) lambda(() maeCreateNetlistForCorner(test c d ?session sess)))
    system(sprintf(nil "cd '%s/netlist' && timeout 600 spectre -format psfascii -raw raw input.scs > spectre.out 2>&1; echo \"exit $?\" >> spectre.out" d))
    nl = chkPLines(strcat(d "/netlist/input.scs"))
    sp = chkPLines(strcat(d "/netlist/spectre.out"))
    chkLine(sprintf(nil "%s includes" c) setof(l nl rexMatchp("^include" l)))
    chkThat(sprintf(nil "%s: the kit's models at section %s" c c)
      exists(l nl rexMatchp(sprintf(nil "^include .*section=%s" c) l)))
    chkLine(sprintf(nil "%s spectre" c) car(last(sp)))
    chkLine(sprintf(nil "%s spectre errors" c) setof(l sp rexMatchp("^ERROR" l)))
    chkThat(sprintf(nil "%s: Spectre runs ADE's netlist" c) car(last(sp)) == "exit 0"))
  chkE("close" lambda(() maeCloseSession(?session sess ?forceClose t))))

;; The OCEAN script as exported, design() naming chkLib, from chkOut.
let((lines port all)
  lines = chkPLines(strcat(chkW "/out/p15_ocean/inv_bench.ocn"))
  port = outfile(strcat(chkOut "/inv_bench.ocn") "w")
  foreach(l lines
    if(rexMatchp("^design" l)
      fprintf(port "design(%L %L %L)\n" chkLib "inv_bench" "schematic")
      fprintf(port "%s\n" l)))
  close(port)
  ;; The work directory's cds.lib INCLUDEs a relative tech.lib: included, not copied.
  system(sprintf(nil "echo 'INCLUDE %s/cds.lib' > '%s/cds.lib' && cd '%s' && timeout 1200 ocean -nograph < inv_bench.ocn > ocean.log 2>&1" chkW chkOut chkOut))
  chkLine("OCEAN errors" setof(l chkPLines(strcat(chkOut "/ocean.log")) rexMatchp("rror" l)))
  system(sprintf(nil "cd '%s' && find . -name '*.txt' | sort > outputs.txt" chkOut))
  foreach(f chkPLines(strcat(chkOut "/outputs.txt"))
    chkLine(sprintf(nil "OCEAN %s" f) chkPLines(strcat(chkOut "/" f)))
    all = cons(buildString(chkPLines(strcat(chkOut "/" f)) " ") all))
  chkThat("OCEAN printed a time for the output"
    rexMatchp("[0-9]e-1[0-9]" buildString(cons("" all) " "))))
chkPass("p_bench ran")
