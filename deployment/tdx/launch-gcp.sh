#!/bin/sh
# Launch one measured guest per independently administered project/placement.
# Requires authenticated gcloud, TDX quota/zone, private subnet and imported guest disk.
set -eu
[ "$#" = 7 ] || { echo 'usage: launch-gcp.sh PROJECT ZONE SUBNET NAME IMAGE MACHINE_TYPE SERVICE_ACCOUNT' >&2; exit 2; }
PROJECT=$1 ZONE=$2 SUBNET=$3 NAME=$4 IMAGE=$5 MACHINE=$6 SERVICE_ACCOUNT=$7
# Actual hardware evidence comes ONLY from native wallet DCAP, never cloud metadata.
gcloud compute instances create "$NAME" --project="$PROJECT" --zone="$ZONE" \
 --subnet="$SUBNET" --no-address --machine-type="$MACHINE" \
 --confidential-compute-type=TDX --maintenance-policy=TERMINATE \
 --image="$IMAGE" --image-project="$PROJECT" --boot-disk-type=pd-balanced \
 --service-account="$SERVICE_ACCOUNT" --no-scopes \
 --no-shielded-secure-boot --shielded-vtpm --shielded-integrity-monitoring
# The unsigned UKI is verified by exact approved MRTD/RTMR policy. Operators can sign
# and enroll the UKI for Secure Boot as a separately approved boot profile.
gcloud compute instances describe "$NAME" --project="$PROJECT" --zone="$ZONE" \
 --format='value(networkInterfaces[0].networkIP)'
