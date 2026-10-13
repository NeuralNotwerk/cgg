# WS14 v2 orchestrator audit (2026-10-13 ~15:40)
Tool: tools/conf_summary.py <abdir> (confidence split + dropped-site fate). User: report A/Bs by confidence and site fate, never raw added/removed.
Regressions sent: (1) PHP 27k medium adds to stub copy noop.php; (2) Scala inherited members refused (QueryCompilationErrors toSQLStmt via QueryErrorsBase→DataTypeErrorsBase); (3) Dart package:flutter_test in-tree not followed.
Checks: (4) JS minified-bundle HIGH churn −35.6k/+56.5k; (5) Python 1,238 HIGH shadowed-by-local drops. Perl drops = fat-comma keys extracted as calls (correct drop; extraction → WS8?). Elixir −2.4k unchecked.
