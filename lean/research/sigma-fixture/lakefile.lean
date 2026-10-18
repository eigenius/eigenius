import Lake
open Lake DSL

/-!
# D74 §4.2 Sigma fixture

The smallest Lean declaration whose type is a `Subtype` in domain position — the shape the DCG
formalizer builds for a refined noun (`ontology.esl`: *"mutator load" -> `Sigma x:Load.
compound_kind(x, Mutator)`*). Names are spelled as D74 §3.3's mangling gives them, so the
externalizer resolves them the way it will against a real #208-generated mirror.

Regenerate `crates/eigenius-lean/test_resources/sigma_subtype.json`:

```sh
cd lean/research/sigma-fixture
lake build
lake exe lean4export SigmaFixture -- refined \
  > ../../../crates/eigenius-lean/test_resources/sigma_subtype.json
```

Path-requires the workspace-vendored `lean4export`, same pin as the capstone project, so the
bytes are reproducible.
-/

package SigmaFixture

require lean4export from "../../runtime-worker/vendor/lean4export"

@[default_target]
lean_lib SigmaFixture where
  roots := #[`SigmaFixture]
