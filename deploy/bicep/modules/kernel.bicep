// Eigenius Kernel Service — ContainerApp

param location string
param environment string
param environmentId string
@minLength(71)
@description('Registry digest of the kernel image, as `sha256:…`. A DIGEST and not a tag: a tag is mutable, so pinning by one makes the deployment irreproducible even in principle, and the kernel writes this value onto every prov:VerificationTrace as its checker identity (D87 §9.3). It is the only identity that binds the running BINARY rather than the source it was built from.')
param imageDigest string
param acrLoginServer string

resource kernelApp 'Microsoft.App/containerApps@2024-03-01' = {
  name: 'eigenius-kernel'
  location: location
  properties: {
    environmentId: environmentId
    configuration: {
      ingress: {
        external: false  // Internal only — orchestration connects via internal DNS
        targetPort: 50051
        transport: 'http2'  // gRPC
      }
      registries: [
        {
          server: acrLoginServer
          identity: 'system'
        }
      ]
    }
    template: {
      containers: [
        {
          name: 'kernel'
          image: '${acrLoginServer}/eigenius-kernel@${imageDigest}'
          resources: {
            cpu: json('1.0')
            memory: '2Gi'
          }
          env: [
            { name: 'EIGENIUS_GRPC_PORT', value: '50051' }
            { name: 'EIGENIUS_HEALTH_PORT', value: '8081' }
            { name: 'EIGENIUS_STORAGE_BACKEND', value: 'sqlite' }
            // D87 §9.3 — the deployer already knows which digest it deployed, and that is where
            // the fact is authoritative: it needs no privilege and works on every runtime. Asking
            // the container runtime instead does not work here at all (ACA has no Docker socket),
            // would require mounting one (root-equivalent on the host), and `.Image` is the local
            // image ID rather than the registry digest a registry actually served.
            { name: 'EIGENIUS_IMAGE_DIGEST', value: imageDigest }
            // TiKV config added in production parameters
          ]
          probes: [
            {
              type: 'Readiness'
              httpGet: {
                port: 8081
                path: '/health'
              }
              initialDelaySeconds: 5
              periodSeconds: 10
            }
          ]
        }
      ]
      scale: {
        minReplicas: environment == 'production' ? 2 : 1
        maxReplicas: environment == 'production' ? 10 : 2
      }
    }
  }
  identity: {
    type: 'SystemAssigned'
  }
}

output fqdn string = kernelApp.properties.configuration.ingress.fqdn
