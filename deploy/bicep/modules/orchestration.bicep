// Eigenius Orchestration Service — ContainerApp

param location string
param environment string
param environmentId string
param kernelFqdn string
param imageTag string
param acrLoginServer string

resource orchestrationApp 'Microsoft.App/containerApps@2024-03-01' = {
  name: 'eigenius-orchestration'
  location: location
  properties: {
    environmentId: environmentId
    configuration: {
      ingress: {
        external: true  // Public-facing API gateway
        targetPort: 8080
        transport: 'http'
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
          name: 'orchestration'
          image: '${acrLoginServer}/eigenius-orchestration:${imageTag}'
          resources: {
            cpu: json('0.5')
            memory: '1Gi'
          }
          env: [
            { name: 'EIGENIUS_KERNEL_ENDPOINT', value: 'http://${kernelFqdn}:50051' }
            { name: 'EIGENIUS_HTTP_PORT', value: '8080' }
            { name: 'EIGENIUS_MCP_PORT', value: '3000' }
          ]
          probes: [
            {
              type: 'Readiness'
              httpGet: {
                port: 8080
                path: '/health'
              }
              initialDelaySeconds: 5
              periodSeconds: 10
            }
          ]
        }
      ]
      scale: {
        minReplicas: 1
        maxReplicas: environment == 'production' ? 10 : 2
      }
    }
  }
  identity: {
    type: 'SystemAssigned'
  }
}

output fqdn string = orchestrationApp.properties.configuration.ingress.fqdn
