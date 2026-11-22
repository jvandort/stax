# Releasing stax

Set the version in two places:
   - `subprojects/flamegraph/build.gradle.kts`
   - `subprojects/flamegraph/src/main/rust/Cargo.toml`

Then update the lock file:
```sh
./gradlew :stax:cargo --cmd="update -w"
```

Build and test everything from scratch:
```sh
./gradlew clean build
```

## Release the Java library

Load the secrets. Close the shell when you're done to clear the secrets from the environment:
```sh
export BW_SESSION="$(bw unlock --raw)"
bw sync

token="$(bw list items --search stax-maven-central-release-token | jq '.[] | select(.name == "stax-maven-central-release-token")')"
key="$(bw list items --search stax-signing-key | jq '.[] | select(.name == "stax-signing-key")')"
export ORG_GRADLE_PROJECT_mavenCentralUsername="$(jq -r .login.username <<<"$token")"
export ORG_GRADLE_PROJECT_mavenCentralPassword="$(jq -r .login.password <<<"$token")"
export ORG_GRADLE_PROJECT_signingKey="$(jq -r .notes <<<"$key")"
export ORG_GRADLE_PROJECT_signingPassword="$(jq -r '.fields[] | select(.name == "password") | .value' <<<"$key")"
unset token key
```

Build the exact bundle that would be uploaded, without uploading it:
```sh
./gradlew nmcpZipAggregation
unzip -l build/nmcp/zip/aggregation.zip
```

It should hold `com/jvandort/stax/<version>/` with the jar, its sources, javadoc and test
fixtures jars, the `.pom` and the `.module`. Each has an `.asc` signature and `.md5`, `.sha1`
and `.sha512` checksums. If the `.asc` files are missing, `signingKey` isn't set.

Upload the bundle to the portal and wait for it to validate:
```sh
./gradlew publishAggregationToCentralPortal
```

Publish or drop the deployment:
Open <https://central.sonatype.com/publishing/deployments>. The deployment shows as **Validated**.
Expand it and look over the files and the POM. Choose **Publish**. That's the point of no return. Or 
choose **Drop** to throw the upload away. You can then fix things and upload the same version again.

Publishing usually takes 10 to 30 minutes. The deployment shows as **Published** when it's done.
