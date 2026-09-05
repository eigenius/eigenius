using '../main.bicep'

param environment = 'production'
param imageTag = 'v0.1.0'
// The digest CI resolved for the image it just pushed — `docker buildx imagetools
// inspect` or the `digest` output of `docker/build-push-action`. There is no default:
// a wrong digest fails the deployment, a defaulted one deploys something nobody chose.
param kernelImageDigest = ''
param acrLoginServer = 'eigeniusacr.azurecr.io'
