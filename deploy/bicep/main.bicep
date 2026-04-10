// Eigenius Azure ContainerApps Deployment
//
// Orchestrates all infrastructure modules for a complete Eigenius deployment.
// Usage: az deployment group create --template-file main.bicep --parameters @parameters/staging.bicepparam

targetScope = 'resourceGroup'

@description('Environment name (staging, production)')
param environment string

@description('Azure region')
param location string = resourceGroup().location

@description('Container image tag')
param imageTag string

@description('ACR login server')
param acrLoginServer string

// --- Container Registry ---
module acr 'modules/acr.bicep' = {
  name: 'acr'
  params: {
    location: location
  }
}

// --- ContainerApps Environment ---
module env 'modules/environment.bicep' = {
  name: 'environment'
  params: {
    location: location
    environment: environment
  }
}

// --- Key Vault ---
module keyvault 'modules/keyvault.bicep' = {
  name: 'keyvault'
  params: {
    location: location
    environment: environment
  }
}

// --- Kernel Service ---
module kernel 'modules/kernel.bicep' = {
  name: 'kernel'
  params: {
    location: location
    environment: environment
    environmentId: env.outputs.environmentId
    imageTag: imageTag
    acrLoginServer: acrLoginServer
  }
}

// --- Orchestration Service ---
module orchestration 'modules/orchestration.bicep' = {
  name: 'orchestration'
  params: {
    location: location
    environment: environment
    environmentId: env.outputs.environmentId
    kernelFqdn: kernel.outputs.fqdn
    imageTag: imageTag
    acrLoginServer: acrLoginServer
  }
}

output kernelFqdn string = kernel.outputs.fqdn
output orchestrationFqdn string = orchestration.outputs.fqdn
