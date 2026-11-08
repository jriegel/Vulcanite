---
title: "Unreal nanit asset pipeline. Welche schritte sind bekannt bis zum File format auf der Platte."
source: "https://www.google.com/search?client=firefox-b-d&q=Unreal+nanit+asset+pipeline.+Welche+schritte+sind+bekannt+bis+zum+File+format+auf+der+Platte.&fbs=ACAhTvx0CAPB6TRWsNnItqaR7Zg8Xd2Ksf0a2MFLySnnxd_WlapqCDG8B3Fe4SzW52WwiJSOZxPXh8odgkG4UnEhmq8V0N2Lj1MVNtXX-EFSw5VBVhKbYgj-OsgA5fWJu_9EOQK1mpcmTeQrIWZpxyuJPhuaIFBK1pP_mHfz5n1q1WY1Lr3F6eSmX8bOYFCLp5vTEdbbMnrg&aep=10&ntc=1&sxsrf=AKL5yDiO8aBg1glosn1l15hc4AzxdUGqfQ%3A1793953820791&mstk=AUtExfDvsghIDCdSnIKqCbswezFh0DI6SlsNo7xTT3VYz7OBlIr-0KCA7VEcU-ahLDc9wf536UGwiOohCqn5P4VQERe0iUHsNCzKmYj_6LLU6ndn9MkUnJ9mrT6m6vlwODCM7LPl5YnEVcwYUHJzrysgVQjvSAIG2WvfM1HSazUVfUvOh-pB8rFHkF9YZHlRlBHuqJNV4SfGv9USZWYgJegOtAao4e-G4nZ1pMoV8h8aoqxrpJmkApCyl_lDypF8HN7-5FVRaIY6YvjnscPSmRhIWWj6duEt5HlUaTQkZeDTwrb3FjdDA_zSUuWnt1hVbLwXt1Rz0nUzKeVzri4Wo2aNysZV8V1GU4YoCVS_IfWGkWnUEx2tnfhgXl-559TYPvrPFRZEnZGElZuqq2qcq3ySMivqT8ZDyAUB1A&aioh=3&csuir=1&mtid=HZDtaqfoB5qAi-gPy-PKuQk&udm=50"
author:
published:
created: 2026-11-06
description:
tags:
  - "clippings"
---
## Unterhaltung im KI-Modus: Unreal nanit asset pipeline. Welche schritte sind bekannt bis zum File format auf der Platte.

## Du hast Folgendes gesagt: Unreal nanit asset pipeline. Welche schritte sind bekannt bis zum File format auf der Platte.

Unreal nanit asset pipeline. Welche schritte sind bekannt bis zum File format auf der Platte.

### Antwort des KI-Modus für „Unreal nanit asset pipeline. Welche schritte sind bekannt bis zum File format auf der Platte.“

Der Weg eines 3D-Modells von der Erstellung bis als fertiges **Nanite-Asset auf der Festplatte** in der Unreal Engine lässt sich in eine klare Kette von Schritten unterteilen.

**Nanite** bricht mit dem traditionellen Ansatz (manuelle LODs, strenge Polygon-Budgets) und setzt stattdessen auf eine virtuelle Geometrie, die Milliarden von Polygonen verarbeiten kann, indem sie Daten intelligent streamt und komprimiert.

Hier ist die Pipeline von der Quelle bis zum Speicherformat:

1\. Datenimport und Vorbereitung

- **Der Import:** Sie importieren ein hochauflösendes Mesh (meist als `.fbx`, `.obj` oder via *Datasmith* / *USD*).
- **Nanite-Aktivierung:** Beim Import oder im Mesh-Editor setzen Sie das Häkchen bei **Enable Nanite**. Ohne diesen Haken durchläuft das Asset die Standard-Static-Mesh-Pipeline.

2\. Clusterisierung (Die Aufteilung)

Sobald Nanite aktiv ist, analysiert die Engine die Geometrie und zerlegt das riesige Mesh in kleine, überschaubare Teile:

- **Cluster-Bildung:** Das Mesh wird in **Cluster von standardmäßig 128 Dreiecken** unterteilt.
- **Konnektivität:** Die Engine berechnet, wie diese Cluster an den Kanten miteinander verbunden sind, damit beim späteren Rendern keine Löcher oder Risse entstehen.

3\. Hierarchischer DAG-Aufbau (Die Pyramide)

Dies ist das Herzstück von Nanite. Die Engine baut einen gerichteten azyklischen Graphen (Directed Acyclic Graph – **DAG**) auf, der wie eine Pyramide funktioniert:

- **Reduktion:** Jeweils ein paar benachbarte Cluster werden zusammengefasst und mathematisch zu einer vereinfachten Version mit weniger Polygonen reduziert (ähnlich einem traditionellen LOD, aber extrem granular).
- **Hierarchie:** Dieser Prozess wiederholt sich, bis an der Spitze nur noch ein sehr grobes Cluster-Paar übrig bleibt.
- **Der Zweck:** Zur Laufzeit entscheidet die Engine pro Frame und auf Basis der Pixelgröße auf dem Bildschirm, welche "Stufe" der Pyramide für jeden einzelnen Cluster geladen werden muss.

4\. Kompression und Quantisierung

Um die enormen Datenmengen auf der Festplatte klein zu halten, werden die Geometriedaten radikal optimiert:

- **Positions-Quantisierung:** Scheitelpunkte (Vertices) werden nicht in standardmäßigen 32-Bit-Fließkommazahlen (Floats) gespeichert, sondern an ein lokales Koordinatennetz angepasst und stark komprimiert.
- **Verlustbehaftete & Verlustfreie Kompression:** Nanite nutzt spezialisierte Kompressionsalgorithmen für die Geometrie, gefolgt von allgemeinen Datenkompressoren (wie *Oodle*, das in der Unreal Engine integriert ist).

5\. Das File-Format auf der Platte (Die Serialisierung)

Wenn Sie das Projekt speichern oder "coken" (für den Release vorbereiten), werden die Daten in den Projektordner geschrieben:

- **Der Asset-Container (`.uasset`):** Wie jedes Static Mesh wird auch ein Nanite-Asset in einer `.uasset` -Datei (im `Content` -Ordner) gespeichert.
- **Die innere Struktur:** Die `.uasset` -Datei ist ein Container. Bei einem Nanite-Asset teilt sich der Inhalt intern auf:
	1. **Header & Metadaten:** Standard-Informationen (Material-Zuweisungen, Kollisionseinstellungen, Name).
		2. **Fallback-Mesh:** Ein traditionelles, stark reduziertes Mesh (meist ca. 2.000 bis 10.000 Polygone). Dieses wird genutzt, wenn die Plattform kein Nanite unterstützt (z. B. ältere Konsolen) oder für komplexe physikalische Berechnungen.
		3. **Nanite Data Stream:** Der eigentliche DAG-Graph und die komprimierten Cluster.
- **Gecooktes Format (`.ubulk` / IoStore):** Wenn das Spiel final exportiert wird (Packaged Build), werden die riesigen Nanite-Streaming-Daten oft aus der Haupt-`.uasset` herausgelöst und in **`.ubulk`** -Dateien oder in die Container des neuen **IoStore-Systems (`.ucas` / `.utoc`)** geschrieben. Dies ermöglicht es der Engine, während des Spielens Daten blitzschnell per DirectStorage oder optimiertem Streaming direkt von der SSD in den Grafikspeicher zu laden.

Möchten Sie wissen, wie Sie die **Nanite-Einstellungen beim Import** (wie die Precision oder das Fallback-Mesh) optimal konfigurieren, oder interessiert Sie, wie der **Streaming-Prozess zur Laufzeit** im Grafikspeicher (VRAM) abläuft?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

Wenn Unreal die assets direkt in den VRAM lädt, werden dann die Kompression auf der GPU dekodiert?

### Antwort des KI-Modus für „Wenn Unreal die assets direkt in den VRAM lädt, werden dann die Kompression auf der GPU dekodiert?“

==**Ja, absolut.**== Das ist einer der Hauptgründe, warum Nanite so extrem schnell ist und gigantische Datenmengen bewältigen kann.

Die Daten werden in ihrer **komprimierten Form von der Festplatte (SSD) direkt in den VRAM** gestreamt und erst **auf der GPU in Compute Shaders dekodiert**.

Das System läuft im Detail wie folgt ab:

Der Dekodierungs- und Rendering-Prozess auf der GPU

1\. Zero CPU-Intervention beim Entpacken

Traditionell mussten 3D-Modelle von der CPU geladen, entpackt, in Standard-Arrays umgewandelt und an die GPU geschickt werden. Nanite umgeht diesen Flaschenhals: Die CPU fordert die Datenblöcke (Cluster) von der SSD an, diese landen komprimiert im VRAM, und die CPU fasst die eigentlichen Geometriedaten nicht mehr an.

2\. On-the-Fly Dekodierung per Compute Shader

Wenn ein Objekt auf dem Bildschirm sichtbar wird, ermittelt die GPU, welche Cluster aus der Hierarchie (dem DAG) benötigt werden.

- **Der Compute Shader:** Speziell geschriebene GPU-Programme (Compute Shader) nehmen die hochgradig quantisierten und bit-komprimierten Cluster-Daten aus dem VRAM.
- **Laufzeit-Entpackung:** Die Bit-Ströme werden direkt im Grafikspeicher bitweise zerlegt (Bit-Unpacking). Aus den lokalen, quantisierten Koordinaten werden in Bruchteilen von Millisekunden wieder echte 3D-Positionen für das Rendering berechnet.

3\. Effiziente VRAM-Nutzung durch Streaming-Pools

Da die Daten komprimiert im VRAM liegen, verbrauchen sie dort deutlich weniger Platz. Unreal reserviert einen festen Speicherpool im VRAM für Nanite (den *Streaming Pool*). Die GPU dekodiert **nur die Cluster, die exakt in diesem Frame gezeichnet werden müssen**. Sobald ein Teil des Objekts die Kamera verlässt oder zu weit entfernt ist, werden die Daten im VRAM einfach durch neue, komprimierte Daten für sichtbare Objekte überschrieben.

4\. Hardware-Beschleunigung (DirectStorage / IoStore)

Unterstützen die Plattform und die Grafikkarte moderne APIs wie **DirectStorage** (auf PC und Xbox Series X/S) bzw. das entsprechende Äquivalent auf der PlayStation 5, kann der Datentransfer sogar noch direkter ablaufen. Die Daten wandern ohne Umwege über den Systemspeicher (RAM) direkt von der NVMe-SSD in den VRAM, wo die GPU-Kompression (wie z. B. die *Oodle Kraken/Leviathan* -Dekodierung auf Hardware-Ebene bei der PS5) die erste Stufe des Entpackens übernimmt, bevor Nanites eigene Geometrie-Shader den Rest erledigen.

Möchten Sie wissen, wie Sie den **Nanite-Streaming-Pool im VRAM** per Konsolenbefehl überwachen und anpassen können, oder interessiert Sie, wie Nanite mit **Overdraw und Software-Rasterisierung** auf der GPU umgeht?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

findet auch die Oodle dekodierung auf der GPU statt?

### Antwort des KI-Modus für „findet auch die Oodle dekodierung auf der GPU statt?“

==**Nein, standardmäßig läuft die Oodle-Dekodierung (Oodle Data) im PC-Bereich auf der CPU**==**.**

Intel Community +1

Hier muss man sehr präzise zwischen zwei verschiedenen Stufen der Kompression trennen, die bei einem Nanite-Asset ineinandergreifen: **Oodle** (die äußere Verpackung) und **Nanites eigene Bit-Kompression** (der innere Inhalt).

LMU München

Das Zusammenspiel teilt sich auf zwei Ebenen auf:

Stufe 1: Das Entpacken des Containers (CPU-Sache)

Wenn die Unreal Engine ein gecooktes Nanite-Asset von der Festplatte anfordert, liegt dieses verpackt in einem Paket (IoStore/`.ucas` /`.utoc`) vor. Dieses Paket ist mit **Oodle Data** (z. B. den Codecs *Kraken* oder *Leviathan*) komprimiert worden, um die Dateigröße auf der Festplatte drastisch zu reduzieren.

StraySpark +1

- **Der Ablauf:** Auf dem PC lädt die CPU diesen Oodle-Datenstrom und dekodiert ihn im Systemspeicher (RAM).
	Epic Games Developers
- **Warum auf der CPU?** Oodle Data ist ein extrem optimierter, verlustfreier Allzweck-Kompressor. Er ist darauf ausgelegt, massiv parallelisiert auf modernen CPU-Kernen mit extrem hoher Geschwindigkeit (mehrere Gigabytes pro Sekunde) zu dekomprimieren.
	RAD Game Tools +1
- *(Interessanter Nebeneffekt: Genau diese extrem hohe CPU-Last bei der Oodle-Shader- und Asset-Dekodierung führte vor einiger Zeit zu den bekannten Spielabstürzen bei instabilen Intel Core i9-Prozessoren der 13. und 14. Generation).*
	RAD Game Tools

Stufe 2: Die Geometrie-Dekodierung (GPU-Sache)

Sobald die CPU die Oodle-Verpackung entfernt hat, liegen die "rohen" Nanite-Cluster-Daten im RAM. Diese Daten sind aber keineswegs unkomprimiert! Sie sind nun in Nanites speziellem, **bit-quantisierten Format** codiert.

LMU München +1

- **Der Ablauf:** Diese immer noch sehr kompakten Geometrie-Seiten werden asynchron über die PCIe-Schnittstelle in den VRAM-Streaming-Pool geschoben.
	Medium·Nikhil Maurya +1
- **Die GPU-Arbeit:** Erst wenn die GPU entscheidet, welche Dreiecks-Cluster gezeichnet werden müssen, greifen die **Compute Shader**. Sie entpacken diese quantisierten Bitströme on-the-fly im Grafikspeicher, um die finalen Vertex-Positionen für den Bildschirm zu berechnen.
	LMU München

---

Die großen Ausnahmen: Konsolen & DirectStorage 1.2+

Obwohl der Standard-Pfad auf dem PC über die CPU läuft, verschwimmt diese Grenze durch moderne Hardware-Architekturen zunehmend:

1. **PlayStation 5 (Hardware-Dekodierung):** Die PS5 besitzt einen dedizierten, physischen Hardware-Chip (Custom Decompressor) für Oodle Kraken. Wenn UE5 auf der PS5 läuft, streamt die Konsole die Oodle-Pakete direkt von der SSD, und der Hardware-Chip entpackt sie ohne jede CPU-Last auf dem Weg in den Speicher.
2. **PC mit DirectStorage 1.2+ (GPU-Decompression):** Microsofts DirectStorage erlaubt es unter Windows, die CPU beim Entpacken zu umgehen. Seit Unreal Engine 5.7+ wird DirectStorage nativ besser integriert. Wenn Entwickler **Oodle Texture** (für GPU-Texturen wie BC1–BC7) nutzen und DirectStorage aktiv ist, können Textur-Pakete tatsächlich direkt auf der GPU über DirectStorage/GDK dekomprimiert werden. Für die reinen *Geometriedaten* (Nanite) bleibt der Pfad auf dem PC aus logistischen Gründen (Paket-Struktur der `.ucas` -Dateien) aber meistens bei der CPU-Vorauswahl.
	Reddit·r/PS5 +2

**Zusammenfassung:** Die Datei wird von der CPU via Oodle entpackt (äußere Hülle), im VRAM abgelegt und die darin enthaltene, hochgradig komprimierte Nanite-Geometrie wird live per Compute Shader auf der GPU dekodiert.

LMU München +2

Möchten Sie wissen, wie **Oodle Texture** (für Texturen) im Vergleich zu **Oodle Data** (für Meshes/Nanite) in Unreal Engine arbeitet, oder interessiert Sie, wie man **DirectStorage** im Projekt aktiviert?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

- LMU München
	Geometry Processing
	Nanite is a GPU-driven deferred rendering pipeline for rigid high-resolution meshes, bypassing traditional polygon budgets for per...
	![](data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wCEAAkGBwgHBgkIBwgKCgkLDRYPDQwMDRsUFRAWIB0iIiAdHx8kKDQsJCYxJx8fLT0tMTU3Ojo6Iys/RD84QzQ5OjcBCgoKDQwNGg8PGjclHyU3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3N//AABEIAIgAiAMBIgACEQEDEQH/xAAbAAACAwEBAQAAAAAAAAAAAAAFBgADBAcBAv/EADkQAAIBAgUCBAQEBgIBBQAAAAECAwQRAAUSITETQQYiUWEUcYGRFTKhsSNCwdHh8AdS8SQlM3KC/8QAFwEBAQEBAAAAAAAAAAAAAAAAAgEAA//EAB8RAAICAgIDAQAAAAAAAAAAAAABAhEhMRJBAzJRIv/aAAwDAQACEQMRAD8A5sxAUse4xhmfWduBi6dyduBjOynm22OUUdpS6PBj3Hg2Fzj6v6b4QDzHlxj03tj4IxjEZ8dR/wCIaRp6WplJY3qLAD+ayj+49McuVbnzY6j/AMd+Io8s8OyQrFG3Td3mL3uSTcfS1h9MSeix2Zf+RKSraOeWbjWbAncAeXYXNhsftjm2x+eOn+JM0hqknhMXTSMnqTkG0hXZrD0DXxy82FrcdvliQVFm7IdI5NvfHRoaivjynL1NBKb0qMSliRsOVvcfUY5ywLIQB27Y6hl+YRy5dSSvOBqj1DuQO18aWjQ2Z56apngjNfNT08GkkkPqltbggC30ucTNFhXLqXoPIiuH1blix0gbg8eny+uMuY59LTEGMCRXX+HZhY/4x807TZmIIYZZCsVwlzY35+hPzwBg/OWamhy+ITpJ0yWJQbFgVN97eltx64bczrpqnMFrZoovPHGb9UOdQY+S3bax9NxgZmWSy1NPS08MNUQxdppkQNsbEH58jcjg4IQ0jy0LLDQpJNDYmpRtwl/ysoNr3v6Eb4r0RbAXiqvlqMiqA7SqGq2LKh1R232P/X5Hm3piY21zSUX4pOsLTBJ4ndDYo3lGzC52+t/2xMKOgvYkuAxGK5BuAMWkWI9sUyk6rDnCCbMpoY6x6jqrI4ihLKsfJbYD6b4HqpUWIsRh08C00JWZ2eMzTSJTrGeVWxZj6emFvNaJaKuqKYTJKIXKiSM3Vvl+2CnkVYB5x8HF2i4xGUKoOEEqHHv6YO+D4Z58zAi1dEW657abjf8AfF/gqPLJa6aLMqGKqLqGiWS5Atzx3t+2HL8MjlzpsryaFaOiR9TrDsdRte5Pc7De9gPTbEfwuTLn8VVmdJmUawxlTT/wIVYHQR5iB8yu/wA8cvZSDZwVI5BFiDjv+Z5XkOX0apLKsUxUgOrMb/Lc/wC9u2OeeJ8uy1urW1VM7ynT/Fgl6eoG4BIIN9youdz9MaLSM0xTyfJKzOUrDRBT8JD1XBJ33sFHud/th0yDK54MpSLO45afXGGijDBWKXI37jttgTkmaTeGMocJThvjmL9U+i2Fvobnf1OGvMauszOjo5PiEilcAyEHUVBH2+mJKRYxoCPS5Zltb+IH4ieOG4WIjWC1iBtb+px85BEk0TnS8UTvZNYIuu4/35Y2fi2XZE7xZgj1MlrfDkjyn1I7H548ybOKetWY0miCWM3CoNBY2Nj+m5522OC1gt26Df4ouW0yTmbQQOgtLKQpaAD8zE2IO1xcegwOzvNKx6GPL6SRHinnZw+jSzb8m1tjviuvzCfrfHVpaSeEHptE5OljYlhfnsMDXzeoMM79Upp0mMqAQpLbW255xzbfR0VLZkzoiOvr4BpePWoDMtjwvA/3bExiq6qesd5ZxIdVj1GH5iNhviYSI6M9XQVdKiNVU0sSuAVZ1IvjIUs4fSThnz/OazPaulSUuwZtMa2ABJNu2A2ct8LmU0SxgWN1A4seP99sdbOdG/wrMIKismKsVhgMtr23Gw/fAGdnklZnJYsbk++DHh2Vm/Fb7Xozx8xhfLE884K9mV+qLj5RvbFTnUp9MV3x9FvLhBGbwDVR0ubSu6qWeneNS3Ym1z9r4bqFJo6iWohY6z5ZoooxcEbHYdu/2J5xzClq5KWaOaJzqQ8Hi3ocP/h3M/jaqCenlamZhaVmFwSOdr73ta+w2wWVBHNK6lqY/OXEoH5dOx/xhVrZRNQmnCBnkkVI9IuxtuR7/wCcNVdleRT10iNU1ita6lQAvf29r4B53Q5bkM5TqVNQ7IDFOgUNEe62Oxvvv22waQslD5zX+HqOCj+EonuraZpo+qb91Hpbba+BdX4nq5qfpU6LBKwOt4gVuSd9P/UHbjH2+fQTZgJKvLg8CNsobzG2wDE/fbvj4kz2IIaeCkXoEG5lOt2JO2/AA9NsXRlkG02Xz1MilmVQ1wDK+kE23Fzt/T3wyZVDl0NSaWgmWWpqAXJjbVGmm+kX7nc7jbC7NUyTqimRnjjXSgfey34/QYIeE2VfEEB34byje+1/3wW8CSyF6yhkpcp1tOpeWU6IjyQOWA9CLfbA3MMwjqKdaWgpAlirM7G2shSP74urER/Enw0i2QyKRddVgeR7+uKPFVO1JmjLFN8TTsoKPps3yYeo9rjERmCZOt5ZKmS4Bsilrj6dsTEqIljq3jq0kMiEq0fBU+ntiYQaC+mTLMzVEdWkhl0hkIO97bH15+uKfE071mbsoGv4dOkvl07L3++CWX+F85mEdakDNH1NXma2pR/N8u+PFoIuvmVZWSpCkheOJ37E33tz/pwuydGXwyh/9yFrgUhHO97j+2F6aJo4oZWPlnDMvrsxX9wcN3hSmumavFJ1QsIF1vY3BsP0wNzVI5sqyqKOn0yRxtutz5R5T92BOIpLkVrAuX82Plz5jg5RZDXVtJWVkCJ0qUF5NThSQN7Ad8CqqndCA43x0TA0Z74dvD1dRplESU7M1XEtjECAym9z23G+3zwk2wd8PUqTQ1tSSyyU8amLQbHUTb6jEmsGixhnzWUzTa6d+qRezqB/XYYweKJ3qUpqp5o5AQBZWuAbA2Hr23xTMry0j1HUFyemRquxB9uPrbGTMlEVFQoB/wB739Q1scUdGCmazEAG1+4x8u5Nuceyvdj5hiu4tuRjqc7LQxYf9cMXgKBZ/E8C9XSFika7WHFh/XC0jr6HBrw7leYVkvxtKEip6cMzzM6gKAN9uf8Azg0NMNeLMqqoq58xpZEPSZSdANxe+5+364w5RHJUZvBVNGZ/4iySMBe1t7/TbbDJBUL8ZVR0sKu4hXUrKbu22m9+/J+oxnhrKkGLp6Y4Gk1GGKMIOdwT3xzdLA1byafGNHT1lFTzrEQtPH+Y/mlu3JsO3piYYkqsvqo2ZZIOvp0tAWsAv/UfW5+uJhJmEoeIMwr66GOUyMrThVGtivPAwPzKvnpcylWIoCrMFLrwPa/zwXy6h6kKVEUyQ2qA8DqRd7uRc77XtffsMCfE9KIs0OlxUSO5DFb2A2/2+2F2DoL+HhnbNPHMsgWpjDpddRksTY23NucAc5jqaVKJKu4k6Tal/wD2xtt88OmX1s8Yq/h5olkUCnvJJpVlFrWYe7H7G/bAHxBCiz08iVME80aMswJ02Ygk2PB5+e3GJizZolEJT4XlgRb/ABbaQt92YMu1/ni8f8a1FRRiaXO6CGZULmDSXKjvex2tfcnjFDSinocrpWl1RHQ7DT+VmNzcd8baqrkMQiqZI5KaeoRnhjQ+a3cn2vxjckiqNoS808PZllYkaeFZYIzvUQOJIz9Rx9R+mDnhmmoJ/D1ZPNRo88TlOsrv1FBG1rGwA+WC2Y5TmVTSVNdAjTxqB8LKXVioFxsDyp439BjB4HpcurMvzOozCFmqYpBd0lZDptsLDbm/bFu0SqB8NHGcjqpnAMy2CNqOoEnjY4z5nRM8OVQksplLKSdzucNOb5XlgyWR6QJ1tWvqEkFQxuAP0wOjpDBS5VLJCWhpJ7MT+dgwPbvv6djgor0LnibLafK8zWmpZXdOkjEyWvci54wKttjp2TZ7FQVFTJT0auXlA8yI7BQvGprWG9/vgXnMK5xTy1cVBG0xQSNJTII+5IW35QQPvbHTl0ChEXvhq8M5lAtCaKVlj0uSTazOpI+9rcYAZbEr5lSxMhIeZIyncgsBb9cdPofBkFDFBWVcK00MsyjpTAmQjVuLc8cfPe2I3k1YL8wrY6PLjmtV0Ipi4WMRbSz+jhewF+Tjn+YZhFmVSi0FBI0wfVeIF2NzvsPW+OhZyPxLMuk+X0oipVMUPUBJCAX4748hoauJClhCjCxNNEE7fzetiMHirsXJ1Qow+G8yCiWpMWXgqWU1EoVzY2IC839jbHmHpjQ0dOk9TLCjNYmRX1tvseCSRt33xMLigtsXMurZKGGeGhpEMDqdJYF7Ktze5+vfvjM8c7QxVssN45YmjCQXUkMRq29LXG3rh3p1oqOjfLoyQjKdQRQGPYW7LvzbAOjekizmgp0dw5jf4iME6TYMQfQMLD5g74PYgBmdDLT0XxKuFjqJS0aB7mMab2JPfcd7/fDDWZZkr5BTa5Clc4V5JXTphnI3B5AO9uTb3wR8QQJNk06RHS8aEAEgNLcAi4tudyO1rY3VOY5YYBRimlhplhtMZAxVABz5uOPXvjR2VidluVtWRI2dfwV1p0mFmAtpBA3HYH6b4bKTwdRQVArGkqZgkmqOEJa+/fvYfr7Y8yHLkoqSjaqWNkC//EynWvJ8x2sbc974LVtTN8NHQo8ohA8wa4JFthzx/bGpWTk6opObJR0PwdNlNQ9LEgTQse6gHsL/AD98c9hyzIqnM6quhMixMh10qM0TK19vy2HHNyd+wx1RqGggpI3hqpNTAE2by3Ptx64BTpFHXxZpBToyyqIplA3kBUMpPuMJsiFzxTQ0y5WZEgEU0MKBTc/ltYC997e4wuZXVV1JJAcsp2mKhwkjLrZbCx39rHbD54pakzSiFPHGoVNIWbvc7fb2xTl8cRkpXaJDohMbsPLpNhYC1vc45xkmxyToH5H4Zoc0F5TVxSPKGMgGkOSDe1/cX2H2wy9GDIoJMtosujmV7axLOvm8oUXN9jYDsBvimOtip44hEnShLEkXuUIB83mJvvt64qovEcNJ8WypTyTrrD9U2J/mDrfkfL1GHgLsB+HPDdJR55USQUsyzW6jwywK3wak+VVYm1z62sB3weWKozGV55HLtcinR2JYKP5gbWNz69iecBvCWYBKCWX+HJUVlQzSGXUemAL3Av235wyJ8XTQBUE4kcjR12Ui7bbWHHf+uKnYX8FmSDOMvzUwyu88DrpV4UIA23GwvYbD6YKxeF6qujklzCadIFP8gFyOwGonv8sGGzD4KnmrYVWd3OiNQoUyAD9P/GL8wzeWGlZnpZSbBunG6m2wPfe/bFpGd0AoPCyx1560FPGgF9KsZDY72N7Ac9h3xMGaGqlzGridqCspY5VADuLDYeh7be2JjWbYow1Mk1ZBVtGL6tOq5RtPFzc2sAdxbGfJsrd82kzWCdaiN45DGGIUq191AJ9Nr8WP0wezmsmgyGQ0qwxOiXkaT+YX819vQ4T6U1FQGgWoVKipGuSYxlrJcWGw29R2uMc1Tyh5WBjeWGoiraiI3Pw48hGlRY3uPvjNmlNJUU9D8NSwuhdBUNEi2hHJ2tsfceuLhB+FZTNLXAWdSqa3Ad+O3rinL8gko8yEdK3SgaIiQSS3Iva5j29CebWIxq/RW8B17yGe3U/jLpCNIbgdwF9QN/pgOc8lqKql+BhsyNpKTjSGVgQBfjsLX2xtq3SnqZ4atWgp0ssRHnBt+UgeoFx9L4HS0+VBGh6lYI3uxKiwsNx8gL+nbEMW18FQGIiSriGq3SekknVT6K0ZsfkcDkzOVavLaKJahYpaoSPPOugzNfTsvZR2Fv0wdymmknpKgxjriNrRFpyGcEXJPa/G17+2EUySVHiWDrCSJkrY4wkgYkea1t+BfG5NmofM6i00h6cQ3YMZF3v9sU0amrhgtCYEJF5JLC2kc2vfexAPHmxrzirenyuoi1hVchSHHvhVjzRqeOWp+FjgsNLy380trAGx7C/b1GB4ti8mgpmbxyI1PW3pl6ltIXXqtvt6kftzjCkMS66akqpujLZHhkjAjkA21X33539sZqWHMZpVqqkqYybQQJKJGt6ixO2C8uUTzQdBoKpFZb6liIsN+PU+3yx1oFmHJdeX1xlmeIR0HUYufNGrbC+3N8HoJpqOmSapgaWRVUtGJvyu5sNP3IxjyKj+G/8ASt8UzK4gkl0EK4PIe/F9/wDO2JJUEOBKJel1wblbX5I2HYc/QYmkbsIpUNUTJAaKSmgpkupdg2o8bHvYeuKsyrgXhZBLMY/M7fD9Ihdux27k3AxsSZXpo2gXVIw1aJPJ6C1zx2wPV5YKqpqIupFOEQea2qPc9xzi8mjUiml8V0lJSSZdS9duqzAOTqeNTyqgd74mBklJSx1jVKTCnfzXkji89z7+5Pt88TBTtFcaZSviGoqitJoBDkIzi1vmMW5QrLUyOCyMH8o0gXAAG3ci5X64E0OdrJUwRvSh5ZX0dUHgHuALfvjfDV1MEizCRXpHjAUHSGVr3I244GJXFYNd7CWf+IZaOOCianTyRlo5HTUNZ787/Uc4H5FmkjdWpzCmLTQ6tUkklr3A7E/M7D6Y+s/nFfSo9K0NwNLrINz6D7nFGYzvUVFNS0nSGojQQuoDgaW2B3PqeBxjRd7NJUfaZvJmNdFGys0UbBlkJBJF7bkAcX/3jBuKJui4A2lKm5/+liPvbFtDQxSxFaelpaKlLfxGjBIdr7WNzsDwBtgz+Ax/CogramxItpCqf2Jw+LYUxBrqrMqGqSUwmXLwOoPJdb2At6bYN5Xm0Gby09NmIlmmV0m6kZROkQbgEEXIv2vxg5XZfPSxXoagksADFJbSVGxttsd+fXCUuUtTyU9dQPI/UnX4zUbaLSE2t2uLfb32z+GQ45nFT1OXVa1TsrdMlWAvdhxhIK/F1NJB0BbZmXWFB0gC9+Rzf3w65vqWgqSF1Kb3Om+keuEupqloUSrenL6HKBxpvCWGzD1bY7HHLxvLOk9DJldYtOsKoqKJGDsU28vZdvb98NMmYUrhvhlKiNd2Y3sx/l+f98I1NUwym0Zc+RfM3JFhY/p+uCiVD9O6uCAeL/09P7Y0vI06IoJo1V1ShmptYkjeRxoJa6m5txvuCMCIJqh6yZKmjkmkpdUrpDLoKJwu/drb22xpqZRDMJ0tUGAqViIA1yH8ltthc378Y0161MGRKZF11blVeRECu5J3J33sN8dH9CgdT5jRV9oYZXkpzGLyzqVEeo7I9+GJv9+2N1O0EElXTy+S6qFMgtZttuSf9+4qGmoxGY5atevVL1jG8FgYhYaW32Nzt6b++N8lDFTZWKqnmarSQgxhzdlAsLE78b7i3GCIreCqClIEhlu1mSUgAgjjfbEx7G9FXMl3nWc2KhGGnsNxz9cTERXbFeDI87mzSPM6rLI6WmSQGYxSLYG3IF+5t2x5kANTRyrOE6SzskLMbDUQTa3HNt+bcYmJjo9nPo3UOXTRGrSoKEaVOnkWvfuCbdu312xdlrTy5hmFRFUxgxjRqTcPZQBvfcb2Pyx7iYiKxjo0EJp4VDGJCLd737/PfDJLmBlkWKEabqGkNh5OwAt67/b3xMTHKM3k6OKBtXMZHKXJN9Sm3HuPf+hF74CPD1J1nbqCOoYRzqrbqwP5rjv/AIxMTHSOTm8BKuqzS088gfQzQshJtYi1t7+uFRaKL8OjqZItdNIGR9Y1HRffQL7b24t684mJgeNfoc9AmvgqKQtMekghtsr7WP8AKL82tuMYpfEM0ZLjR1QLdTSbsP8Ab4mJjs4pg5Ma55HrBl6pKamQ2CrY6dWnyi3bc4Zc1SKjy81Fa7dOGO7RooBZha5+4xMTElokcsTaHKazxHUfidazKtQraULWJQe/pxb74YqNo6Wo+C1t8OH1JrsCxI3CgDccfX649xMSSqKYou3RfUzLR5l8Ogj+DcdQFQAVuNRvbe9ziYmJgop//9k=)
- StraySpark
	DirectStorage 1.2 in UE 5.7: What Indies Need to Know About...
	17.04.2026 — Oodle compression — your shipped game's IO already looks the way DirectStorage wants it to look. Texture streaming where compresse...
- Epic Games Developers
	How Nanite Streaming Works: CPU, GPU, & Performance...
	Video Memory (VRAM): Contains the dedicated Nanite Streaming Pool, where requested geometry clusters are allocated for active rend...
	![](data:image/jpeg;base64,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)
- Intel Community
	Intel i9 13900KF causing Epic Games crash
	10.05.2024 — The underlying cause is that Oodle which is Epic's Unreal Engine integrated compressor/decompressor utility is sensitive to Intel...
- RAD Game Tools
	Oodle Features
	Compression Codecs: Leviathan achieves LZMA-comparable compression with 10x faster decompression. Kraken maximizes decode speed; D...
	![](https://encrypted-tbn0.gstatic.com/images?q=tbn:ANd9GcQUwmGf168Jqd4wG7IcP4v2hghGaWa-dO6o5VdSkXBybfm455KXZ_w1tHm3oFG2R_3HhqOQI88z_K5_uubyXDMBeXcT_dBu-mm2UrteDpE)
- RAD Game Tools
	Intel Processor Instability Causing Oodle Decompression Failures
	14th generation Intel Core processors. System instability, Oodle Data decompression shader failures in Unreal Engine games, and cr...
- Medium·Nikhil Maurya
	Nanite in Unreal Engine 5: A Technical Breakdown for Artists and Developers | Medium
	GPU-driven, cluster-based virtualized geometry system using 128-triangle clusters and automatic LOD hierarchies.
	![](https://encrypted-tbn2.gstatic.com/images?q=tbn:ANd9GcR1yGW7njPlgk1gL4J52nZ1OrVjVSI8lA7rwk3MQXGw5BinbEr-Ni7t_r5cZ-i5I-je46JEYqtp_y1gl3m4td2bvDGgDoCB9ABAkmLn0hw)
- Reddit·r/PS5
	Oodle data compression solutions now available in Unreal...
	It means that any game built using Unreal Engine can now use Oodle compression tools for free. hardware-based decompression for Oo...

Alle anzeigen

Kann man DirectStorage mit wgpu (WebGPU) nutzen, oder braucht man dazu Vulkan oder DX12?

### Antwort des KI-Modus für „Kann man DirectStorage mit wgpu (WebGPU) nutzen, oder braucht man dazu Vulkan oder DX12?“

==Nein, **DirectStorage kann derzeit nicht direkt mit WebGPU (`wgpu`) genutzt werden.**== Um DirectStorage zu verwenden, ist zwingend eine native Low-Level-API wie **DirectX 12 (DX12)** oder **Vulkan** erforderlich.

Hier ist die technische Begründung, warum das so ist und welche Alternativen es in der WebGPU-Welt gibt:

Warum DirectStorage DX12 oder Vulkan benötigt

DirectStorage ist eine native Microsoft-Technologie, die extrem tief in das Betriebssystem (Windows) und den Grafiktreiber integriert ist.

- **API-Bindung:** Die offizielle DirectStorage-API von Microsoft gibt Daten direkt in **DX12-Ressourcen** (wie `ID3D12Resource`) aus.
- **Vulkan-Äquivalent:** Für Vulkan gibt es spezielle Erweiterungen (wie `VK_NV_cooperative_matrix` oder herstellerspezifische Erweiterungen für DirectStorage-ähnliche Pfade), die eine ähnliche Zero-Copy-Pipeline ermöglichen.
- **Das Problem mit WebGPU / `wgpu`:** WebGPU ist eine **Abstraktionsschicht**. Das bedeutet, `wgpu` (in Rust/C++) oder die WebGPU-API im Browser übersetzt Befehle im Hintergrund in DX12, Vulkan oder Metal. Da WebGPU jedoch plattformunabhängig und im Web sandboxed sein muss, bietet es (bisher) keine Schnittstelle, um native Betriebssystem-Handles von DirectStorage an die darunterliegende DX12/Vulkan-Ressource durchzureichen.

Der Flaschenhals in WebGPU heute

Wenn Sie in WebGPU (z. B. mit der Rust-Bibliothek `wgpu`) Daten von der Festplatte in den VRAM laden möchten, müssen Sie den traditionellen Weg gehen:

1. Datei über das Betriebssystem in den normalen Arbeitsspeicher (**RAM**) laden.
2. Die Daten in einen WebGPU-Staging-Buffer schreiben.
3. Über `queue.write_buffer()` oder `copy_buffer_to_buffer()` die Daten von der CPU über die PCIe-Schnittstelle in den **VRAM** kopieren.

Da WebGPU im Browser extrem strengen **Sicherheits- und Sandbox-Regeln** unterliegt, hat es keinen direkten Zugriff auf Low-Level-Dateisystem-APIs, die für DirectStorage (wie das Überspringen des CPU-Kernels via `BypassIO`) notwendig wären.

Gibt es Ausnahmen für "Natives WebGPU" (`wgpu` in Rust)?

Wenn Sie `wgpu` nicht im Browser, sondern als native Desktop-Anwendung (z. B. in Rust) nutzen, theoretisch: **Nein, auch da nicht direkt.**

Allerdings erlaubt `wgpu` über sogenannte *Hal-Passthroughs* oder unsichere Erweiterungen (Unsafe Features) manchmal den Zugriff auf die darunterliegenden nativen Objekte (z. B. das rohe `ID3D12Device` von DX12). Sie müssten dann jedoch:

- WebGPU-Abstraktionen umgehen.
- DirectStorage komplett manuell in C++/Rust anbinden.
- Das Ergebnis händisch in die internen Texturen/Buffer von `wgpu` reinpfuschen. Das bricht das gesamte Konzept von WebGPU und führt oft zu Synchronisationsfehlern.

Die WebGPU-Alternative: Asynchrones Streaming & Web-APIs

Wenn Sie in WebGPU große Datenmengen (wie für ein eigenes Nanite-ähnliches System) streamen möchten, nutzt man statt DirectStorage moderne Web-Standards, die zumindest die CPU-Last minimieren:

- **Storage Foundation API / Origin Private File System (OPFS):** Erlaubt extrem schnellen, sequentiellen Lesezugriff auf exklusive Dateien im Browser.
- **Chunk-basiertes Laden mit Web Workern:** Das Laden und eventuelle Dekomprimieren (z. B. via WebAssembly) wird auf Hintergrund-CPU-Threads ausgelagert, damit das Rendering (der Haupt-Thread) nicht ruckelt.

**Fazit:** Wenn Ihr Ziel maximale NVMe-Performance und GPU-Dekompression via DirectStorage ist, führt derzeit kein Weg an **DirectX 12** oder **Vulkan** vorbei. WebGPU ist für ein plattformübergreifendes, sicheres Ökosystem gedacht und unterstützt solche proprietären Low-Level-Speicherpfade aktuell nicht.

Planen Sie ein **eigenes Software-Rasterizer- oder Geometrie-Streaming-Projekt** in Rust/ `wgpu`, oder ging es Ihnen um die Frage, ob man ein System wie Nanite effizient ins **Web / in den Browser** portieren kann?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

Kannst du mir noch mehr über die Quantisierung und die Bitstream optimierung von Nanite sagen?

### Antwort des KI-Modus für „Kannst du mir noch mehr über die Quantisierung und die Bitstream optimierung von Nanite sagen?“

Um Milliarden von Polygonen ohne gigantischen VRAM-Bedarf zu rendern, nutzt Nanite eine hochentwickelte **Verlustbehaftete Kompression (Lossy Compression)** direkt bei der Geometrie. Das Ziel ist es, die Daten so stark zu verpacken, dass sie bitweise komprimiert im VRAM liegen und von den GPU Compute Shadern mit minimalen Bit-Operationen entpackt werden können.

Hier ist der tiefe Einblick in die mathematischen Kniffe und die Bitstream-Architektur von Nanite:

1\. Die Quantisierung (Positions-Komprimierung)

Traditionell speichert die Unreal Engine jeden Vertex-Punkt (X, Y, Z) als drei 32-Bit-Fließkommazahlen (`float32`). Das macht **96 Bit pro Vertex** allein für die Position. Nanite wirft dieses System komplett über Bord.

- **Lokale Bounding Boxen (Lokale Räume):** Jedes Nanite-Mesh wird in kleine Cluster von 128 Dreiecken zerlegt. Jedes dieser Cluster erhält seine eigene, winzige 3D-Begrenzungsbox (Bounding Box).
- **Vom Float zum Integer (Feste Raster):** Statt globaler Fließkommazahlen speichert Nanite die Positionen der Vertices als **Integer-Werte (Ganzzahlen)** relativ zur Bounding Box des Clusters. Der Raum innerhalb des Clusters wird in ein superfeines, diskretes Gitter (Grid) unterteilt.
- **Variable Bitrate pro Cluster:** Nanite analysiert die Geometrie des Clusters. Wenn ein Cluster sehr flach ist (z. B. ein Wandstück), reicht eine geringe Auflösung (z. B. 5 bis 8 Bit pro Achse). Wenn das Cluster extrem detailliert ist, werden bis zu 12 oder 15 Bit genutzt.
- **Das Ergebnis:** Aus 96 Bit für eine Position werden oft **nur noch 15 bis 30 Bit insgesamt** für X, Y und Z zusammen. Da das Gitter so fein ist, sieht das menschliche Auge keinen Unterschied zum Original-Mesh (Verlustfreiheit in der Wahrnehmung).

2\. Attribut-Quantisierung (Normale und UVs)

Nicht nur Positionen, auch Normalen (für die Beleuchtung) und Texturkoordinaten (UVs) fressen normalerweise viel Speicher.

- **Normalen im Oktaeder-Raum:** Traditionelle Normalen-Vektoren benötigen drei Floats. Nanite projiziert die 3D-Richtungsvektoren auf die Oberfläche eines Oktaeders (eine achtseitige Pyramide) und faltet diese mathematisch in eine 2D-Ebene aus. Diese 2D-Koordinate wird dann oft in nur **2x 8-Bit (oder 2x 10-Bit) Integers** abgespeichert.
- **UV-Deltas:** Oft nutzen benachbarte Vertices in einem Cluster sehr ähnliche Texturkoordinaten. Nanite speichert häufig nur die Differenz (das Delta) zum vorherigen Vertex, was dramatisch weniger Bits benötigt als die absoluten UV-Werte.

3\. Die Bitstream-Optimierung (Wie es im VRAM liegt)

Wenn die Daten quantisiert sind, liegen sie als Ketten von unregelmäßigen Bit-Längen vor (z. B. ein Vertex-Wert hat 7 Bit, der nächste 11 Bit). Um dies für die GPU lesbar zu machen, nutzt Nanite ein maßgeschneidertes **Bit-Packing**:

- **Kein Byte-Alignment:** Im normalen Speicher muss jede Variable an Byte-Grenzen (8, 16, 32 Bit) ausgerichtet sein. Nanite ignoriert das komplett. Die Bits werden ohne Lücken direkt hintereinander in den Speicher geschrieben (ein echter Bitstream).
- **Bit-Layout pro Cluster:** Am Anfang jedes Datenblocks eines Clusters steht ein kleiner **Header**. Dieser Header verrät der GPU die "Spielregeln" für dieses spezifische Cluster: *"In diesem Block ist X = 7 Bit groß, Y = 6 Bit, Z = 8 Bit und die UVs haben 5 Bit."*
- **GPU Hardware-Freundlichkeit:** GPUs sind eigentlich schlecht darin, variable Bit-Längen zu lesen (sie lieben 32-Bit-Blöcke). Nanites Bitstream ist so strukturiert, dass ein Compute Shader mittels schneller Bit-Verschiebungsbefehle (`Variable Bit Extraction` / `BFI` / `BFE` Shader-Instruktionen) die exakten Bits für einen Vertex isolieren kann, ohne benachbarte Daten zu beschädigen.

4\. Index-Kompression (Dreiecks-Topologie)

Ein wichtiges Element fehlt noch: Wie sind die Vertices zu Dreiecken verbunden? Normalerweise sagt eine Index-Liste: "Verbinde Vertex 0, 5 und 12".

- Nanite nutzt die Tatsache aus, dass die 128 Dreiecke innerhalb eines Clusters extrem nah beieinanderliegen.
- Es sortiert die Vertices so an, dass aufeinanderfolgende Dreiecke fast immer dieselben Vertices wiederverwenden (Striping/Clustering).
- Statt globaler IDs speichert Nanite für die Dreiecks-Definition oft nur winzige **5-Bit-Indizes**, die sich rein auf die maximal ~256 Vertices *innerhalb* dieses einen Clusters beziehen.

Die Bilanz auf der Festplatte und im VRAM

Durch diese Kombination aus **lokaler Quantisierung, Oktaeder-Projektion und lückenlosem Bit-Packing** schafft es Nanite, die Datenmenge eines hochentwickelten Meshes auf einen Bruchteil zu komprimieren. Wenn der Compute Shader das Asset zur Laufzeit aufruft, liest er den Header des Clusters, wendet die Bit-Masken an, multipliziert die Integer-Werte mit der Skalierung der Bounding Box und erhält in Nanosekunden die exakten 3D-Koordinaten für die Rasterisierung.

Möchten Sie tiefer in den mathematischen Trick der **Oktaeder-Mapping-Projektion** für die Normalen eintauchen, oder interessiert Sie, wie die **Compute Shader** diese Bitströme parallelisiert für 128 Dreiecke gleichzeitig entpacken?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

ja, erzähl mehr über Oktaeder-Mapping-Projektion.

### Antwort des KI-Modus für „ja, erzähl mehr über Oktaeder-Mapping-Projektion.“

Das **Oktaeder-Mapping (Octahedral Mapping)** ist einer der elegantesten mathematischen Tricks in der modernen Grafikprogrammierung. Es wird in Nanite (und vielen Deferred Rendering Engines) genutzt, um eine **3D-Normale (Richtungsvektor mit X, Y, Z) verlustarm in eine 2D-Koordinate (U, V) zu falten**.

Anstatt drei 32-Bit-Floats (96 Bit) oder drei 8-Bit-Integers (24 Bit mit starkem Präzisionsverlust) zu nutzen, komprimiert das Oktaeder-Mapping eine Normale in **zwei 8-Bit- oder 10-Bit-Werte (16–20 Bit)**, behält dabei aber eine extrem hohe visuelle Genauigkeit.

Hier ist die genaue mathematische und geometrische Funktionsweise:

---

Das Problem mit der Kugel

Eine Normale ist ein Einheitsvektor. Das bedeutet, ihre Länge ist immer 1 (x² + y² + z² = 1). Geometrisch betrachtet liegen alle möglichen Normalen also auf der Oberfläche einer **Einheitskugel**.

Wenn man eine Kugeloberfläche flach auf ein 2D-Gitter projizieren will (wie bei einer Weltkarte), hat man immer das Problem von starken Verzerrungen oder mathematischen Singularitäten (den Polen). Das Oktaeder-Mapping löst dieses Problem, indem es die Kugel durch ein **Oktaeder** (eine Doppelpyramide mit 8 dreieckigen Flächen) ersetzt.

---

Der Algorithmus: In 3 Schritten von 3D zu 2D

Schritt 1: Die Projektion auf das Oktaeder

Zuerst wird der 3D-Vektor auf die Oberfläche eines Oktaeders projiziert. Mathematisch wird der Vektor so skaliert, dass die Summe der absoluten Werte seiner Komponenten genau 1 ergibt:  
$\text{Summe} = \left|x\right| + \left|y\right| + \left|z\right|$  
Der Vektor wird durch diese Summe geteilt. Dadurch wandert der Punkt von der Kugeloberfläche exakt auf die Oberfläche des Oktaeders.

Schritt 2: Das "Falten" der oberen Hemisphäre (z ≥ 0)

Blickt man von oben auf das Oktaeder, sieht man eine Pyramide mit einer quadratischen Grundfläche. Wenn die Normale nach oben zeigt (z ist positiv oder null), ist die 2D-Projektion denkbar einfach: Man nimmt einfach die x- und y-Koordinaten. Die quadratische Basis der Pyramide entspricht nun perfekt dem zweidimensionalen Koordinatenraum von

⁠

.

Schritt 3: Das "Umklappen" der unteren Hemisphäre (z < 0)

Zeigt die Normale nach unten (z ist negativ), liegt der Punkt auf der unteren Pyramide. Um diesen auf dieselbe 2D-Fläche zu bringen, wird das Oktaeder mathematisch wie ein Pappkarton **aufgeschnitten und umgeklappt**:

- Die vier unteren Dreiecke werden an den Außenkanten der oberen Pyramide nach oben geklappt.
- Mathematisch spiegelt man die Punkte an den Diagonalen des 2D-Quadrats. Ein Punkt, der über den Rand hinausgehen würde, wird wieder nach innen gefaltet.

Das Ergebnis ist ein perfektes, quadratisches 2D-Grid, in dem **jeder Punkt auf der gesamten 3D-Kugel einen eindeutigen Platz** gefunden hat.

---

Die Bit-Quantisierung und der Shader-Code

Sobald der Vektor im 2D-Raum (mit Werten zwischen -1 und 1) vorliegt, wird er quantisiert. Nanite skaliert diesen Bereich meist auf Ganzzahlen (Integers) – zum Beispiel in ein **8-Bit-Raster (0 bis 255)** für u und v.

Wenn die GPU den Nanite-Cluster rendert, nimmt der Compute Shader diese zwei Bytes und rekonstruiert die 3D-Normale in folgendem, extrem schnellen Prozess:

```
// Sehr vereinfachter Shader-Code zur Rekonstruktion (Unpacking)
float3 DecodeOctahedralNormal(float2 uv) {
    // 1. Zurück in den Bereich [-1, 1] bringen
    float3 n = float3(uv.x, uv.y, 1.0 - abs(uv.x) - abs(uv.y));
    
    // 2. Wenn z negativ war, die Kanten wieder zurückklappen
    if (n.z < 0.0) {
        n.xy = (1.0 - abs(n.yx)) * (n.xy >= 0.0 ? 1.0 : -1.0);
    }
    
    // 3. Den Vektor wieder auf die Kugeloberfläche normalisieren
    return normalize(n);
}
```

---

Warum macht Nanite das?

1. **Massive Bandbreiteneinsparung:** Anstatt 12 Byte (96 Bit) pro Vertex für eine unkomprimierte Normale über den Speicherbus zu jagen, liest die GPU nur 2 Byte (16 Bit). Das reduziert die VRAM-Bandbreite für Geometrie um über **80 %**.
2. **Homogene Verteilung:** Im Gegensatz zu anderen Methoden (wie einfachen sphärischen Koordinaten mit Längengraden/Breitengraden) verteilt das Oktaeder-Mapping die mathematische Genauigkeit extrem gleichmäßig über die gesamte Kugel. Es gibt keine "Verpixelung" oder Artefakte an den Polen eines Objekts.
3. **GPU-Effizienz:** Die mathematischen Operationen beim Entpacken (`abs`, Multiplikationen, Vergleiche) gehören zu den absolut billigsten Befehlen, die eine GPU ausführen kann. Es gibt keine teuren trigonometrischen Funktionen wie `sin()` oder `cos()`. Der Compute Shader erledigt das in wenigen Taktzyklen.

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

wie die Compute Shader diese Bitströme parallelisiert für 128 Dreiecke gleichzeitig entpacken?

### Antwort des KI-Modus für „wie die Compute Shader diese Bitströme parallelisiert für 128 Dreiecke gleichzeitig entpacken?“

Das Entpacken dieser unregelmäßigen Bitströme für **128 Dreiecke** gleichzeitig ist ein Meisterwerk der parallelen GPU-Programmierung. Auf modernen GPUs (wie AMDs RDNA oder NVIDIAS Ada Lovelace/Blackwell) arbeiten Hunderte von Kernen in sogenannten **Warps** (NVIDIA, 32 Threads) oder **Waves/Wavefronts** (AMD, 32 oder 64 Threads) synchron zusammen.

Nanite nutzt das Prinzip der **SIMD-Architektur** (Single Instruction, Multiple Data) perfekt aus, um die Bitketten ohne Schleifen oder Verzögerungen direkt in die Register zu jagen.

So läuft die Parallelisierung im Compute Shader ab:

1\. Das Thread-Mapping: Ein Thread pro Element

Wenn Nanite ein Cluster verarbeitet, startet es eine Thread-Gruppe. Die gängigste Aufteilung in Unreal Engine 5 sieht so aus:

- Ein **Nanite-Cluster besteht aus bis zu 128 Dreiecken und maximal 256 Vertices**.
- Die Thread-Gruppe im Compute Shader besteht meist aus **128 Threads**.
- **Die Zuweisung:** Jeder Thread innerhalb der Wavefront übernimmt genau eine Aufgabe. Im ersten Schritt entpacken die Threads die Vertex-Positionen (Thread 0 entpackt Vertex 0, Thread 1 entpackt Vertex 1 usw.). Im zweiten Schritt entpacken sie die Index-Daten der Dreiecke.

2\. Der "Bit-Streamer" im gemeinsamen Speicher (LDS / Shared Memory)

Da die Daten komprimiert als ein einziges, lückenloses Band von Bits (Bitstream) auf der Festplatte und im VRAM liegen, können die Threads nicht einfach wild irgendwo reinspringen. Sie müssen wissen, wo ihr Wert anfängt.

- **LDS-Caching:** Zuerst laden einige wenige Threads die rohen 32-Bit-Wörter (Dwords) des komprimierten Clusters aus dem VRAM in den extrem schnellen **Local Data Share (LDS)** – den On-Chip-Zwischenspeicher der GPU.
- **Der Header-Broadcast:** Der Shader liest den winzigen Header des Clusters. Dieser enthält die Information, wie viele Bits jede Komponente hat (z.B. *Position-X = 9 Bit*). Diese Info wird an **alle Threads gleichzeitig** übertragen (Broadcast).

3\. Parallele Adressberechnung ohne Schleifen

Jetzt kommt der mathematische Trick: Wenn Thread #5 sein X-Attribut auslesen will, muss er wissen, an welchem exakten Bit im Stream sein Wert beginnt.

- Da jeder Thread die feste Bit-Breite (B) aus dem Header kennt, kann die Start-Bit-Adresse für jeden Thread i über eine simple Multiplikation berechnet werden:  
	$\text{Bit}-\text{Offset} = i \times B$
- **Kein sequentielles Suchen:** Jede GPU-Ader berechnet diesen Offset für sich selbst in einem einzigen Taktzyklus parallel. Thread 0 sucht bei Bit 0, Thread 1 bei Bit 9, Thread 2 bei Bit 18 usw.

4\. Bit-Tricks auf Hardware-Ebene (`BFE` / Bit Field Extract)

GPUs besitzen spezialisierte Hardware-Einheiten für Bit-Operationen. Im Shader-Code wird ein Befehl namens **`BitFieldExtract` (BFE)** aufgerufen.

1. Der Thread greift in das LDS-Array an die Stelle, an der sein berechneter Bit-Offset liegt. Da ein 9-Bit-Wert über die Grenze eines 32-Bit-Wortes ragen kann, lädt der Thread zwei benachbarte 32-Bit-Wörter.
2. Die GPU führt den `BFE` -Befehl aus: `BitFieldExtract(Daten, StartBit, BitLaenge)`.
3. Die Hardware isoliert die gewünschten Bits in Bruchteilen einer Nanosekunde und schiebt sie per Bit-Shift nach rechts.

Aus den rohen Bits im Strom wird so sofort eine isolierte Ganzzahl (Integer) im Register des Threads.

5\. Kollaboratives Dekomprimieren

Da ein Cluster maximal 256 Vertices, aber die Thread-Gruppe oft nur 128 Threads hat, arbeitet die Wavefront in **zwei schnellen Durchgängen (Passes)**:

- **Pass 1:** Threads 0–127 entpacken die Vertices 0–127.
- **Pass 2:** Dieselben Threads entpacken die Vertices 128–255.
- Danach wechseln die Threads ihre Rolle und entpacken die **Dreiecks-Indizes** (Topologie). Da die Indizes (wie vorhin erwähnt) durch Nanites Optimierung oft nur 5 Bit groß sind, lassen sich mit einem einzigen 32-Bit-LDS-Zugriff gleich mehrere Indizes auf einmal für ein Dreieck entpacken.

Warum das so extrem schnell ist

Der entscheidende Vorteil dieses Designs ist das **Fehlen von divergenten Pfaden (Branch Divergence)**. Weil alle Vertices innerhalb *eines* Clusters mit exakt derselben Bit-Breite komprimiert sind, führen alle 128 Threads in der Wavefront in jedem Taktzyklus **haargenau denselben Befehl** aus. Es gibt keine `if-else` -Verzweigungen, die eine GPU ausbremsen würden.

Wenn die Wavefront fertig ist (was oft nur wenige Dutzend Taktzyklen dauert), liegen alle 128 Dreiecke komplett entpackt im ultraschnellen Register-Speicher der GPU bereit und können direkt an Nanites eigenen **Software-Rasterizer** (ebenfalls ein Compute Shader) übergeben werden, um auf den Bildschirm gezeichnet zu werden.

Möchten Sie als Nächstes erfahren, wie Nanites **Software-Rasterizer** diese entpackten Dreiecke ohne die klassische Hardware-Grafikpipeline direkt in den Buffer zeichnet, oder interessiert Sie, wie die **Kollisionsabfrage** mit solchen hochkomprimierten Clustern umgeht?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

Mich interesiert wie Unreal die Nanite meshes im laufenden Betrieb managed. Wie wird entschieden wann welche meshes geladen und entladen werden? Wie wird der GPU und CPU speicher gemanged?

### Antwort des KI-Modus für „Mich interesiert wie Unreal die Nanite meshes im laufenden Betrieb managed. Wie wird entschieden wann welche meshes geladen und entladen werden? Wie wird der GPU und CPU speicher gemanged?“

Das Speichermanagement von Nanite im laufenden Betrieb ist ein ==hochdynamisches **Streaming-System**, das im Grunde wie ein virtueller Texturspeicher für Geometrie funktioniert== (ähnlich wie Virtual Texturing, nur für Dreiecke). Das System arbeitet komplett **datengetrieben, asynchron und budgetorientiert**.

Hier ist die genaue Aufteilung, wie CPU, GPU und die Streaming-Logik im Hintergrund zusammenarbeiten.

---

1\. Die Streaming-Logik: Wer entscheidet, was geladen wird?

Die Entscheidung, welche Detailstufe (welcher Teil des DAG-Graphen) geladen werden muss, wird **auf der GPU getroffen**, aber die Ausführung der Ladebefehle wird an die **CPU übergeben**.

- **Der Sichtbarkeitstest (GPU Cull Phase):** In jedem Frame testet die GPU alle potenziell sichtbaren Nanite-Instanzen. Dabei berechnet ein Compute-Shader für jedes Cluster, wie groß seine Dreiecke auf dem Bildschirm (in Pixeln) wären.
- **Der Streaming-Request:** Stellt die GPU fest, dass eine Kamera näher an ein Objekt heranrückt und die aktuellen Cluster zu grob werden (ein Cluster-Fehler bzw. *Quantisierungs-Fehler* sichtbar würde), markiert sie die benötigten, detaillierteren Cluster-IDs.
- **Die Feedback-Queue:** Diese IDs werden in eine Feedback-Warteschlange geschrieben. Am Ende des Frames liest die CPU diese Warteschlange aus dem VRAM aus. Die GPU sagt der CPU also: *"Ich brauche für das nächste Frame dringend die Cluster-Daten mit den IDs X, Y und Z!"*

---

2\. Das CPU-Speichermanagement (Der Dirigent)

Die CPU verwaltet die Datenhaltung auf der Festplatte (SSD) und im normalen Arbeitsspeicher (RAM).

- **Kein Blockieren (Asynchrones Laden):** Sobald die CPU die Wunschliste der GPU erhält, stößt sie über den **IoStore** (das UE-eigene I/O-System) asynchrone Leseanforderungen an. Die Daten werden in Chunks (oft als *Pages* bezeichnet, ca. 128 KB groß) von der SSD in den RAM gestreamt.
- **Priorisierung:** Da Nanite-Daten hierarchisch aufgebaut sind, priorisiert die CPU immer die gröberen Stufen (die Basis-Pyramide). Wenn die SSD nicht schnell genug hinterherkommt, rendert die GPU einfach die gröbere Version weiter – das Spiel ruckelt nicht (keine "Spikes"), das Asset ploppt im schlimmsten Fall nur sichtbar auf (Texture/Geometry Pop-in).
- **Der RAM-Cache:** Im RAM hält die CPU einen flüchtigen Cache der entpackten Oodle-Container bereit, um sie direkt für den Transfer zur Verfügung zu haben.

---

3\. Das GPU-Speichermanagement (Die Streaming-Pools)

Im VRAM arbeitet Nanite nicht mit dynamischen Allokationen (das wäre viel zu langsam und würde den Speicher fragmentieren). Stattdessen nutzt Unreal zwei riesige, feste Speicherbereiche.

A. Der Nanite Streaming Pool (Byte-Address Buffer)

Beim Spielstart reserviert die Unreal Engine einen festen Block im VRAM (standardmäßig oft zwischen 1 und 2 GB, konfigurierbar über `r.Nanite.Streaming.PoolSize`).

- **Das Seiten-System (Paging):** Dieser Pool ist in gleich große Speicherseiten (Pages) unterteilt.
- **Der GPU-Upload:** Wenn die CPU die angeforderten Cluster-Daten von der SSD geladen und entpackt hat, kopiert sie diese per PCIe-Transfer exakt in die freien Pages dieses Streaming-Pools im VRAM. Die Daten liegen dort, wie zuvor beschrieben, in ihrer bit-komprimierten Form.
- **LRU-Cache (Least Recently Used):** Wenn der Pool voll ist und neue Cluster geladen werden müssen, wirft die Engine die Speicherseiten aus dem VRAM, die am längsten nicht mehr auf dem Bildschirm sichtbar waren.

B. Der Dynamic Scratch Buffer (Der Arbeitsspeicher der GPU)

Da die Daten komprimiert im Pool liegen, benötigt die GPU Platz, um sie während des Renderings zu verarbeiten. Hierfür gibt es einen separaten *Scratch Buffer*.

- Die Compute-Shader lesen die benötigten Pages aus dem Streaming-Pool, dekomprimieren die Geometrie und schreiben die temporären Vertex-Daten direkt in diesen schnellen Grafik-Arbeitsspeicher, wo sie sofort gerastert werden.

---

4\. Das Entladen: Wie wird aufgeräumt?

Nanite löscht Assets fast nie hart aus dem Speicher, es sei denn, ein ganzes Level wird entladen.

- **Hierarchisches Zurückfallen:** Wenn Sie sich von einem gigantischen Nanite-Berg entfernen, fordert die GPU keine detaillierten Pages mehr an. Die detaillierten Seiten bleiben so lange im VRAM-Pool liegen, bis der Platz für ein anderes Objekt benötigt wird.
- **Geringe Bandbreite beim Entfernen:** Da das System auf dem LRU-Prinzip basiert, kostet das "Entladen" keine CPU- oder GPU-Zyklen. Eine Seite wird einfach als "überschreibbar" markiert. Erst wenn neue Daten reinkommen, werden die alten Bits überschrieben.

Zusammenfassendes Datenspiel im Betrieb:

```
[1. GPU misst Pixelgröße] 
       │
       ▼
[2. GPU meldet Request an CPU] ──(PCIe)──► [3. CPU lädt Page von SSD & entpackt Oodle]
                                                   │
                                                 (PCIe)
                                                   │
[5. GPU Compute Shader] ◄──(Liest aus)─── [4. VRAM Nanite Streaming Pool (1-2 GB)]
  Entpackt Bitstream & rastert Live
```

Durch diese strikte Trennung von **fester Pool-Größe im VRAM** und **asynchronem CPU-Streaming** kann die Unreal Engine garantieren, dass die Framerate stabil bleibt, selbst wenn der Spieler mit 300 km/h durch eine Welt fliegt, die aus Hunderten Gigabytes an Roh-Geometrie besteht.

Möchten Sie wissen, wie man diese **Streaming-Pools über Konsolenbefehle (cvars) optimiert**, wenn der Speicher knapp wird, oder interessiert Sie, wie **Nanites Software-Rasterizer** die Dreiecke im Scratch-Buffer schließlich zeichnet?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

Heist das das Nanite von einem mesh nicht immer alle auflösungen lädt? Und wie wird es gemacht das in der Entfernung mehrere grobe meshes zu einem noch gröberen mesh zusammengefasst werden?

### Antwort des KI-Modus für „Heist das das Nanite von einem mesh nicht immer alle auflösungen lädt? Und wie wird es gemacht das in der Entfernung mehrere grobe meshes zu einem noch gröberen mesh zusammengefasst werden?“

**Ja, exakt!** Nanite lädt **niemals das gesamte Mesh mit allen Detailstufen** in den VRAM. Es lädt immer nur die exakten Speicherseiten (Pages), die für die aktuelle Entfernung und Sichtbarkeit benötigt werden.

Stehen Sie direkt vor einer riesigen, detailreichen Statue, lädt Nanite nur die hochauflösenden Cluster für die Nase und das Gesicht. Die Rückseite der Statue oder die Füße (wenn sie außerhalb des Bildschirms sind) werden gar nicht erst in den Grafikspeicher gestreamt.

Die Magie, wie in der Entfernung mehrere grobe Meshes zu einem einzigen, noch gröberen Mesh verschmelzen, liegt in der **hierarchischen Struktur des DAG (Directed Acyclic Graph)** und einem Prozess namens **Cluster-Merging** begründet.

So funktioniert diese Zusammenfassung im Detail:

Der hierarchische DAG-Aufbau (Die Cluster-Pyramide)

Wenn Unreal ein Asset importiert und die Nanite-Daten berechnet, baut sie eine Pyramide von unten nach oben. Stellen Sie sich ein Mesh vor, das im Original aus **10.000 Clustern** (Stufe 0, maximale Details) besteht:

1. **Cluster-Paarung:** Die Engine sucht nach benachbarten Clustern, die räumlich eng zusammenliegen. Sie nimmt beispielsweise vier Cluster der Stufe 0 (insgesamt 4 × 128 = 512 Dreiecke).
2. **Merging & Vereinfachung (Decimation):** Diese vier Cluster werden virtuell zu einer einzigen Geometrie zusammengefügt. Nun jagt die Engine einen hochentwickelten Reduktions-Algorithmus darüber. Dieser reduziert die 512 Dreiecke um exakt die Hälfte – auf **256 Dreiecke**.
3. **Neue Cluster-Teilung:** Diese verbleibenden 256 Dreiecke werden nun wieder in zwei brandneue Cluster der **Stufe 1** (jeweils wieder 128 Dreiecke) aufgeteilt.

Dieser Prozess wiederholt sich über viele Stufen hinweg nach oben hin weg. An der Spitze der Pyramide (Stufe X) existieren vielleicht nur noch zwei extrem grobe Cluster für das gesamte Objekt.

Der Geniestreich: Die gemeinsame Schnittstelle (Locking Boundaries)

Damit die GPU zur Laufzeit nahtlos zwischen diesen Stufen wechseln kann, ohne dass Risse oder Löcher in der Geometrie entstehen, nutzt Nanite einen mathematischen Trick beim Reduzieren:

- Wenn die vier Cluster zusammengefasst und reduziert werden, **bleiben die äußeren Kanten (die Ränder der Gruppe) absolut unangetastet**.
- Nur die Geometrie im *Inneren* der Gruppe wird vereinfacht.
- Dadurch ist garantiert, dass eine hochauflösende Stufe eines Nachbar-Clusters immer noch perfekt an die reduzierte Stufe des aktuellen Clusters andocken kann.

Das Zusammenfassen in der Praxis (Zur Laufzeit)

Wenn sich die Kamera nun von dem Objekt entfernt, passiert auf der GPU Folgendes:

1. Die GPU merkt, dass die vier detaillierten Cluster (Stufe 0) auf dem Bildschirm zusammengerechnet nur noch wenige Pixel groß sind.
2. Anstatt nun vier separate Streaming-Requests zu senden, schaltet der Auswahl-Shader (LOD-Selection) auf die übergeordneten zwei Cluster (Stufe 1) um.
3. **Der Speichereffekt:** Die vier detaillierten Seiten im VRAM-Streaming-Pool werden nicht mehr abgefragt und können vom Speichermanager freigegeben werden. Stattdessen werden die zwei viel kleineren Stufe-1-Seiten aktiv genutzt.

Was passiert bei weit entfernten Instanzen (z. B. Wäldern)?

Wenn Sie ein riesiges Feld voller Nanite-Bäume oder eine Stadt aus Nanite-Häusern aus der Ferne betrachten, greift ein weiteres System: **Instanced Hierarchy Merging**.

Nanite kann nicht nur *innerhalb* eines einzelnen Objekts Cluster zusammenfassen, sondern zieht diese Logik auch über verschiedene Objekt-Instanzen hinweg. Wenn 50 Bäume in der Ferne stehen und jeder Baum an seiner Pyramidenspitze nur noch aus 1–2 Clustern besteht, kann Nanite diese entfernten Spitzen-Cluster im Speicher zusammenfassen und als einen einzigen, kombinierten Geometrieblock behandeln. Für die GPU bricht der Rendering-Aufwand dadurch in der Ferne fast vollständig zusammen.

Möchten Sie wissen, wie Sie im Editor über die **Nanite-Visualisierungs-Ansichten** (wie *Clusters* oder *LOD Error*) diese Pyramiden-Stufen farblich sichtbar machen können, um die Pipeline besser zu verstehen?

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

Wie werden instanzen vom selben mesh an verschiedenen Stellen gehandhabt?

### Antwort des KI-Modus für „Wie werden instanzen vom selben mesh an verschiedenen Stellen gehandhabt?“

Wenn Sie dasselbe Nanite-Mesh hunderte oder tausende Male in Ihrer Welt platzieren (z. B. Steine, Bäume, Säulen oder Stühle), nutzt Unreal Engine 5 ein extrem effizientes System, das auf **GPU-driven Instancing** basiert.

Dabei wird strikt zwischen den **Geometriedaten** und den **Instanzdaten** getrennt.

Das Prinzip funktioniert im laufenden Betrieb wie folgt:

1\. Einmalige Geometrie, viele Transformationen

Die komprimierten Geometrie-Seiten (die Cluster-Pyramide / der DAG), die wir in den vorherigen Schritten besprochen haben, existieren **nur ein einziges Mal im VRAM-Streaming-Pool**. Es ist völlig egal, ob Sie den Stein 1-mal oder 100.000-mal in der Welt platrieren – der Geometrie-Speicherbedarf im VRAM erhöht sich für die Kopien um exakt **0 Byte**.

Für jede einzelne Platzierung (Instanz) in der Welt speichert Unreal auf der CPU und GPU lediglich ein Minimum an Metadaten (die sogenannten **Instanzdaten**):

- Die **Transformations-Matrix** (Position, Rotation, Skalierung).
- Optionale benutzerdefinierte Daten (z. B. Farbvariationen oder ein zufälliger Wert für den Shader).

Diese Instanzdaten sind winzig (nur wenige Bytes pro Instanz) und liegen in einem fortlaufenden, flachen GPU-Buffer (Structured Buffer).

2\. Unabhängige LOD-Auswahl pro Instanz (Per-Instance Culling)

Obwohl alle Instanzen dieselbe Geometrie-Quelle nutzen, entscheidet die GPU für **jede Instanz individuell**, welche Cluster-Auflösung gerade benötigt wird.

Ein Compute Shader läuft über alle Instanzen drüber und führt folgendes aus:

- **Instanz 1 (Direkt vor der Kamera):** Die GPU berechnet, dass dieses Objekt sehr groß auf dem Bildschirm ist. Der Compute Shader fordert die hochauflösenden Cluster (die Basis der Pyramide) aus dem VRAM-Pool an.
- **Instanz 2 (Ganz weit hinten):** Die GPU berechnet, dass diese Instanz nur wenige Pixel groß ist. Der Compute Shader greift für genau diese Instanz einfach auf die gröbsten Cluster an der Spitze der Pyramide zurück.

Da alle Stufen der Pyramide im selben VRAM-Streaming-Pool liegen, liest die GPU einfach je nach Entfernung der Instanz andere Speicheradressen aus.

3\. Draw-Call-Kombination via Multi-Draw Indirect (MDI)

Bei traditionellem Rendering war jede Instanzgruppe oft ein eigener "Draw Call" (Befehl von der CPU an die GPU). Wenn Sie 100 verschiedene Instanz-Typen hatten, zwingt das die CPU in die Knie.

Nanite nutzt **Multi-Draw Indirect**:

1. Ein GPU Compute Shader sammelt in jedem Frame alle sichtbaren Instanzen und deren benötigte Cluster auf.
2. Er filtert diejenigen heraus, die hinter Wänden liegen (Occlusion Culling) oder außerhalb des Sichtfelds der Kamera sind (Frustum Culling).
3. Die verbleibenden, tatsächlich sichtbaren Cluster aller Instanzen werden in einer einzigen, flachen Liste auf der GPU gesammelt.
4. Ein einziger, gigantischer Rendering-Befehl zeichnet schließlich **alle Cluster aller unterschiedlichen Instanzen im gesamten Level auf einmal**.

4\. Was passiert, wenn die Materialien unterschiedlich sind?

Sie können verschiedenen Instanzen desselben Nanite-Meshs unterschiedliche Materialien zuweisen (z. B. ein sauberes und ein moosiges Material). Nanite löst dies, indem es Material-IDs direkt in den Clustern oder Instanzdaten speichert. Der Nanite-Rasterizer zeichnet zunächst alle Geometrien in einen speziellen Zwischenpuffer (den *Visibility Buffer* / *V-Buffer*). Erst in einem späteren Schritt (Material Pass) wird geschaut, welches Pixel welches Material benötigt. Dadurch verliert Nanite auch bei Material-Variationen keine Performance.

**Zusammenfassung:** Nanite behandelt Instanzen als reine "Positions-Stempel". Die GPU schaut auf den Stempel, berechnet die Entfernung, greift sich blitzschnell die passenden, komprimierten Cluster-Häppchen aus dem zentralen Geometrie-Pool und wirft alles in einen einzigen, hocheffizienten Rendering-Durchgang.

KI-Antworten können Fehler enthalten. [Weitere Informationen](https://support.google.com/websearch?p=aimode)

In Zwischenablage kopiertFehler beim Kopieren in die Zwischenablage. Versuche es später noch einmal.

## Geteilt

0 Dateien

Alle anzeigen

## Geteilt

0 Dateien

[Zum vorherigen Prompt](#)

Wird transkribiert...

Google-Apps

Google-Konto

Jürgen Riegel

juergen.riegel6820@googlemail.com