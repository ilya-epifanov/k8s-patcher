## K8S patcher mutating admission webhook controller

### Installation

    helm repo add patcher https://ilya-epifanov.github.io/k8s-patcher/
    helm repo update

    helm install patcher patcher/patcher --wait --create-namespace --namespace patcher --set replicaCount=2

### Usage

