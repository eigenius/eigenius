// Eigenius Kernel Service — ContainerApp

param location string
param environment string
param environmentId string
param imageTag string
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
          image: '${acrLoginServer}/eigenius-kernel:${imageTag}'
          resources: {
            cpu: json('1.0')
            memory: '2Gi'
          }
          env: [
            { name: 'EIGENIUS_GRPC_PORT', value: '50051' }
            { name: 'EIGENIUS_HEALTH_PORT', value: '8081' }
            { name: 'EIGENIUS_STORAGE_BACKEND', value: 'sqlite' }
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
