# Ktor
-keep class io.ktor.** { *; }
-dontwarn io.ktor.**

# Room
-keep class * extends androidx.room.RoomDatabase
-keep @androidx.room.Entity class *
-dontwarn androidx.room.paging.**

# Kotlinx Serialization
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.AnnotationsKt
-keepclassmembers class kotlinx.serialization.json.** { *** Companion; }
-keepclasseswithmembers class kotlinx.serialization.json.** { kotlinx.serialization.KSerializer serializer(...); }
-keep,includedescriptorclasses class com.nola.**$$serializer { *; }
-keepclassmembers class com.nola.** { *** Companion; }
-keepclasseswithmembers class com.nola.** { kotlinx.serialization.KSerializer serializer(...); }

# PDF import extracts text only. PDFBox's optional JPX decoder is not bundled;
# JPXFilter checks Class.forName and raises MissingImageReaderException if absent.
# https://github.com/TomRoush/PdfBox-Android#optional-dependencies
-dontwarn com.gemalto.jp2.JP2Decoder

# SLF4J 1.7 deliberately uses its NOP fallback without an installed binding.
# https://www.slf4j.org/manual.html
-dontwarn org.slf4j.impl.StaticLoggerBinder
