Seguimiento de la condición 1 de ADR-0001 (palancas 4 y 5 de §3 de docs/testing/f1-decision-cond1.md).
Análisis en docs/testing/f1-diagnostico-colas.md §9. Máquina compartida con otros paquetes: cada fila de los
*.jsonl lleva load1/load5/load15 al empezar la ejecución; los intervalos de load1 de cada serie son:

walprealloc.jsonl             agent_memory medium, kerno / kerno-tuned[wal_preallocate=true] / fjall / btree, 7 rep.   load1 2,57-6,07
walprealloc-b.jsonl           lo mismo con el orden invertido (kerno-tuned primero), 8 rep.                           load1 1,35-1,77
walprealloc-t3.jsonl          livespec y cauce medium, lsm / kerno / kerno-tuned[wal_preallocate=true], 7 rep.        load1 1,27-1,98
walsync-writeback.jsonl       microbanco de fdatasync con escritura de fondo (sync_file_range sin espera), 7 rep.     load1 1,94-3,05
walsync-writeback-paced.jsonl ídem con una espera por rango anterior ("con ritmo"), 7 rep.                            load1 2,12-2,74
walsync-writeback-blocking.jsonl ídem esperando el mismo rango ("bloqueante"), 6 rep. completas                       load1 1,73-2,22
ab-writeback*.jsonl           A/B de bytes_per_sync=none,bytes_per_writeback=N frente al valor por defecto de entonces load1 1,20-2,02
ab-dynamic.jsonl              A/B de dynamic_base_level=true                                                          load1 1,33-1,97
ab-minoverlap.jsonl           A/B de file_picker=min_overlap                                                          load1 1,47-1,85
ab-combo.jsonl                A/B de bytes_per_writeback + dynamic_base_level                                         load1 1,39-2,22
attr-livespec.jsonl           atribución del p99 de commit de LiveSpec a cada palanca nueva (8 rep. por variante)     load1 0,71-1,54
results.jsonl                 T1-T5, T7, T8: kerno (valores finales) / kerno-tuned[valores anteriores] / lsm / btree / fjall, 5 rep.  load1 1,14-2,38
large.jsonl                   bulk_load large, 3 rep.                                                                 load1 1,23-2,59
env.json                      entorno de la sesión final
