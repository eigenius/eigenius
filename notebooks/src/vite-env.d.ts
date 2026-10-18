/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_EIGENIUS_ORCHESTRATOR?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
