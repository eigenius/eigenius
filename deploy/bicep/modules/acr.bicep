// Azure Container Registry

param location string

resource acr 'Microsoft.ContainerRegistry/registries@2023-07-01' = {
  name: 'eigeniusacr'
  location: location
  sku: {
    name: 'Basic'
  }
  properties: {
    adminUserEnabled: false
  }
}

output loginServer string = acr.properties.loginServer
