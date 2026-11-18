import org.gradle.api.file.DuplicatesStrategy
import org.gradle.api.tasks.bundling.Zip
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile
import java.io.File

plugins {
    alias(libs.plugins.kotlin.jvm)
    application
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

tasks.withType<KotlinCompile>().configureEach {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

application {
    mainClass.set("com.touchpad.windows.WindowsMainFrameKt")
}

dependencies {
    implementation(project(":shared-protocol"))
    implementation("net.java.dev.jna:jna:5.6.0")
    testImplementation("junit:junit:4.13.2")
}

tasks.withType<Test>().configureEach {
    systemProperty("java.awt.headless", "true")
}

tasks.jar {
    archiveFileName.set("windows-app.jar")
    manifest {
        attributes["Main-Class"] = "com.touchpad.windows.WindowsMainFrameKt"
    }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    exclude("META-INF/*.SF", "META-INF/*.DSA", "META-INF/*.RSA")
    dependsOn(configurations.runtimeClasspath)
    from({
        configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) }
    })
}

val jpackageStagingDir = layout.buildDirectory.dir("jpackage-input")
val portableDistRoot = rootProject.layout.projectDirectory.dir("dist")
val portableAppImageDir = portableDistRoot.dir("TouchPadRemote")

val preparePortableInput by tasks.registering(Copy::class) {
    dependsOn(tasks.jar)
    from(tasks.jar.flatMap { it.archiveFile })
    into(jpackageStagingDir)
}

val buildPortableAppImage by tasks.registering {
    dependsOn(preparePortableInput)
    inputs.files(tasks.jar.flatMap { it.archiveFile })
    outputs.dir(portableAppImageDir)

    doLast {
        val jpackageCandidates = listOfNotNull(
            System.getenv("JAVA_HOME")?.let { File(it, "bin/jpackage.exe") },
            File("C:/Program Files/Java/jdk-22/bin/jpackage.exe")
        )
        val jpackageExe = jpackageCandidates.firstOrNull { it.isFile }
            ?: error("jpackage.exe não encontrado. Verifique a instalação do JDK 22.")

        val outDir = portableAppImageDir.asFile
        if (outDir.exists()) {
            outDir.deleteRecursively()
        }
        portableDistRoot.asFile.mkdirs()

        project.exec {
            commandLine(
                jpackageExe.absolutePath,
                "--type", "app-image",
                "--name", "TouchPadRemote",
                "--app-version", "1.0.0",
                "--vendor", "TouchPad Remote",
                "--input", jpackageStagingDir.get().asFile.absolutePath,
                "--main-jar", "windows-app.jar",
                "--main-class", "com.touchpad.windows.WindowsMainFrameKt",
                "--dest", portableDistRoot.asFile.absolutePath,
                "--add-modules", "java.base,java.desktop,java.logging,java.management,jdk.unsupported",
                "--java-options", "-Dfile.encoding=UTF-8"
            )
        }

        File(outDir, "Liberar-Firewall-Windows.bat").writeText(
            """
            @echo off
            setlocal
            set "EXE_PATH=%~dp0TouchPadRemote.exe"
            echo Solicitando permissao de Administrador para liberar o TouchPadRemote no Firewall do Windows (UDP 45454)...
            powershell -NoProfile -Command "Start-Process cmd -Verb RunAs -ArgumentList '/c netsh advfirewall firewall delete rule name=\"TouchPad Remote\" >nul 2>&1 & netsh advfirewall firewall add rule name=\"TouchPad Remote\" dir=in action=allow program=\"%EXE_PATH%\" enable=yes profile=any & netsh advfirewall firewall add rule name=\"TouchPad Remote UDP 45454\" dir=in action=allow protocol=UDP localport=45454 enable=yes profile=any'"
            echo Pronto! Agora abra o TouchPadRemote.exe normalmente.
            pause
            endlocal
            """.trimIndent().replace("\n", "\r\n"),
            Charsets.UTF_8
        )

        File(outDir, "LEIA-ME.txt").writeText(
            """
            TouchPad Remote - Versao Portatil Windows (nao precisa instalar Java!)

            1. Copie a pasta "TouchPadRemote" inteira (ou extraia o .zip) no outro computador Windows.
            2. Execute "TouchPadRemote.exe".
            3. Se o Firewall do Windows perguntar, clique em "Permitir acesso".
               (Se o celular mostrar Offline mesmo aberto, execute "Liberar-Firewall-Windows.bat" uma vez).
            4. No aplicativo do celular, cadastre o IP exibido na janela (porta 45454) ou use "Buscar na Rede".
            """.trimIndent().replace("\n", "\r\n"),
            Charsets.UTF_8
        )
    }
}

val packagePortableWindows by tasks.registering(Zip::class) {
    dependsOn(buildPortableAppImage)
    archiveFileName.set("TouchPadRemote-Windows-Portatil.zip")
    destinationDirectory.set(portableDistRoot)
    from(portableDistRoot) {
        include("TouchPadRemote/**")
    }
}
