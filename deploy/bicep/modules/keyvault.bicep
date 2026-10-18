// Azure Key Vault for secrets management

param location string
param environment string

resource keyVault 'Microsoft.KeyVault/vaults@2023-07-01' = {
  name: 'eigenius-${environment}-kv'
  location: location
  properties: {
    sku: {
      family: 'A'
      name: 'standard'
    }
    tenantId: subscription().tenantId
    enableRbacAuthorization: true
  }
}

output vaultUri string = keyVault.properties.vaultUri
