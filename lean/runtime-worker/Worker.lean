/-
Copyright 2026 The Eigenius Authors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-/

/-!
# `Worker` — Lean root module for the Eigenius runtime worker

Re-exports the public surface so consumers (the `lean-runtime-worker`
executable's `Worker.Main` and any future test fixtures) can write
`import Worker` instead of cherry-picking submodules.
-/

import Worker.Ffi
