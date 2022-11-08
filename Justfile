cluster_name := "patcher"
docker_user := "smartislav"
docker_image := "k8s-patcher"

cluster-up:
    kind create cluster --name {{cluster_name}} --image kindest/node:v1.23.5 --config kind-config.yaml
    sleep "1"
    kubectl wait --namespace kube-system --for=condition=ready pod --selector="tier=control-plane" --timeout=180s

deploy-cert-manager:
    helm repo add jetstack https://charts.jetstack.io
    helm repo update
    helm install cert-manager jetstack/cert-manager --namespace cert-manager --create-namespace --version v1.8.0 --set installCRDs=true

build:
    docker build -t {{docker_user}}/{{docker_image}} .

load:
    kind --name {{cluster_name}} load docker-image {{docker_user}}/{{docker_image}}:latest

deploy:
    helm install patcher charts/patcher-chart --wait --create-namespace --namespace patcher --set image.tag=latest --set replicaCount=2

debug:
    kubectl apply -f deploy/debug.yaml

cluster-down:
    kind delete cluster --name {{cluster_name}}

all: cluster-up deploy-cert-manager build load deploy

delete:
    helm delete patcher --wait --namespace patcher
    kubectl delete -f deploy/debug.yaml || /bin/true
