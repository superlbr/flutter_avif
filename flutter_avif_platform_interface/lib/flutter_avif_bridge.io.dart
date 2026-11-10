import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:ffi';
import 'dart:isolate';
import 'dart:typed_data';
import 'package:ffi/ffi.dart';

import 'package:flutter_avif_platform_interface/flutter_avif_ffi.dart'
    as fa_ffi;
import 'package:flutter_avif_platform_interface/models/avif_info.pb.dart';
import 'package:flutter_avif_platform_interface/models/encode_request.pb.dart';
import 'package:flutter_avif_platform_interface/models/frame.pb.dart';
import 'package:flutter_avif_platform_interface/models/key_request.pb.dart';
import 'package:flutter_avif_platform_interface/models/encode_frame.pb.dart';
import 'package:protobuf/protobuf.dart';
export 'package:flutter_avif_platform_interface/models/encode_frame.pb.dart';

class FlutterAvifImpl implements FlutterAvif {
  static const int PROTOBUF_SIZE_LIMIT = 2147483648;

  final fa_ffi.FlutterAvifFFI flutterAvifFFI;

  FlutterAvifImpl(ffi.DynamicLibrary dyLib)
      : this.flutterAvifFFI = fa_ffi.FlutterAvifFFI(dyLib) {
    final storeDartPostCObject =
        dyLib.lookupFunction<dartPostCObject, dartPostCObject>(
      'store_dart_post_cobject',
    );
    storeDartPostCObject(NativeApi.postCObject);
  }

  @override
  Future<Frame> decodeSingleFrameImage({required Uint8List avifBytes}) {
    final request = KeyRequest(
      key: "",
      data: avifBytes,
    ).writeToBuffer();
    final nativeRequest = toPointer(request);

    final completer = Completer<Frame>();
    final port = RawReceivePort();
    port.handler = (response) {
      port.close();

      final frame = decodeFrameResponse(response as List<int>);

      completer.complete(frame);
    };

    flutterAvifFFI.decode_single_frame_image(
      port.sendPort.nativePort,
      nativeRequest[0],
      nativeRequest[1],
    );

    malloc.free(nativeRequest[0]);

    return completer.future;
  }

  @override
  Future<bool> disposeDecoder({required String key}) {
    final request = KeyRequest(
      key: key,
      data: Uint8List.fromList([1]),
    ).writeToBuffer();
    final nativeRequest = toPointer(request);

    final completer = Completer<bool>();
    final port = RawReceivePort();
    port.handler = (response) {
      port.close();

      completer.complete(response);
    };

    flutterAvifFFI.dispose_decoder(
      port.sendPort.nativePort,
      nativeRequest[0],
      nativeRequest[1],
    );

    malloc.free(nativeRequest[0]);

    return completer.future;
  }

  @override
  Future<Uint8List> encodeAvif({
    required int width,
    required int height,
    required int speed,
    required int maxThreads,
    required int timescale,
    required int maxQuantizer,
    required int minQuantizer,
    required int maxQuantizerAlpha,
    required int minQuantizerAlpha,
    required List<EncodeFrame> imageSequence,
    required Uint8List exifData,
  }) {
    final request = EncodeRequest(
      width: width,
      height: height,
      speed: speed,
      maxThreads: maxThreads,
      timescale: timescale,
      maxQuantizer: maxQuantizer,
      minQuantizer: minQuantizer,
      maxQuantizerAlpha: maxQuantizerAlpha,
      minQuantizerAlpha: minQuantizerAlpha,
      imageList: imageSequence,
      exifData: exifData,
    ).writeToBuffer();
    final nativeRequest = toPointer(request);

    final completer = Completer<Uint8List>();
    final port = RawReceivePort();
    port.handler = (response) {
      port.close();

      completer.complete(response);
    };

    flutterAvifFFI.encode_avif(
      port.sendPort.nativePort,
      nativeRequest[0],
      nativeRequest[1],
    );

    malloc.free(nativeRequest[0]);

    return completer.future;
  }

  @override
  Future<Frame> getNextFrame({required String key}) {
    final request = KeyRequest(
      key: key,
      data: Uint8List.fromList([1]),
    ).writeToBuffer();
    final nativeRequest = toPointer(request);

    final completer = Completer<Frame>();
    final port = RawReceivePort();
    port.handler = (response) {
      port.close();
      
      final frame = decodeFrameResponse(response as List<int>);

      completer.complete(frame);
    };

    flutterAvifFFI.get_next_frame(
      port.sendPort.nativePort,
      nativeRequest[0],
      nativeRequest[1],
    );

    malloc.free(nativeRequest[0]);

    return completer.future;
  }

  @override
  Future<AvifInfo> initMemoryDecoder({
    required String key,
    required Uint8List avifBytes,
  }) {
    final request = KeyRequest(
      key: key,
      data: avifBytes,
    ).writeToBuffer();
    final nativeRequest = toPointer(request);

    final completer = Completer<AvifInfo>();
    final port = RawReceivePort();
    port.handler = (response) {
      port.close();

      final info = AvifInfo.create();
      final codedInput = CodedBufferReader(response, sizeLimit: PROTOBUF_SIZE_LIMIT);
      info.mergeFromCodedBufferReader(codedInput);

      completer.complete(info);
    };

    flutterAvifFFI.init_memory_decoder(
      port.sendPort.nativePort,
      nativeRequest[0],
      nativeRequest[1],
    );
    malloc.free(nativeRequest[0]);

    return completer.future;
  }

  @override
  Future<bool> resetDecoder({required String key}) {
    final request = KeyRequest(
      key: key,
      data: Uint8List.fromList([1]),
    ).writeToBuffer();
    final nativeRequest = toPointer(request);

    final completer = Completer<bool>();
    final port = RawReceivePort();
    port.handler = (response) {
      port.close();

      completer.complete(response);
    };

    flutterAvifFFI.reset_decoder(
      port.sendPort.nativePort,
      nativeRequest[0],
      nativeRequest[1],
    );

    malloc.free(nativeRequest[0]);

    return completer.future;
  }
}

abstract class FlutterAvif {
  Future<Frame> decodeSingleFrameImage({required Uint8List avifBytes});

  Future<AvifInfo> initMemoryDecoder({
    required String key,
    required Uint8List avifBytes,
  });

  Future<bool> resetDecoder({required String key});

  Future<bool> disposeDecoder({required String key});

  Future<Frame> getNextFrame({required String key});

  Future<Uint8List> encodeAvif({
    required int width,
    required int height,
    required int speed,
    required int maxThreads,
    required int timescale,
    required int maxQuantizer,
    required int minQuantizer,
    required int maxQuantizerAlpha,
    required int minQuantizerAlpha,
    required List<EncodeFrame> imageSequence,
    required Uint8List exifData,
  });
}

class DecodeData {
  final Uint8List data;
  final Uint32List durations;
  final int width;
  final int height;

  DecodeData({
    required this.data,
    required this.durations,
    required this.width,
    required this.height,
  });
}

/// 解析 native 回传的 Frame 响应，**像素字节零拷贝**。
///
/// 生成的 `mergeFromCodedBufferReader` 对 bytes 字段走
/// `CodedBufferReader.readBytes()`（内部 `Uint8List.fromList`），会把 1080×2346
/// 的 RGBA（~9.7MB）从回传缓冲里**再复制一份**。这里按 Frame 的四个字段手写解析，
/// 字段 1 用 `readBytesAsView()` 直接返回回传缓冲的视图 ⇒ 峰值只保留一份。
/// 代价：`frame.data` 的生命周期与回传缓冲绑定（视图），调用方用完即弃即可。
Frame decodeFrameResponse(List<int> response) {
  final input = CodedBufferReader(
    response,
    sizeLimit: FlutterAvifImpl.PROTOBUF_SIZE_LIMIT,
  );
  final frame = Frame.create();
  while (true) {
    final tag = input.readTag();
    if (tag == 0) break;
    final field = getTagFieldNumber(tag);
    final wire = getTagWireType(tag);
    switch (field) {
      case 1: // data: bytes
        if (wire == WIRETYPE_LENGTH_DELIMITED) {
          frame.data = input.readBytesAsView();
        } else {
          input.skipField(tag);
        }
        break;
      case 2: // duration: double
        if (wire == WIRETYPE_FIXED64) {
          frame.duration = input.readDouble();
        } else {
          input.skipField(tag);
        }
        break;
      case 3: // width: uint32
      case 4: // height: uint32
        if (wire == WIRETYPE_VARINT) {
          final value = input.readUint32();
          if (field == 3) {
            frame.width = value;
          } else {
            frame.height = value;
          }
        } else {
          input.skipField(tag);
        }
        break;
      default:
        input.skipField(tag);
    }
  }
  return frame;
}

List toPointer(Uint8List units) {
  final ffi.Pointer<ffi.Uint8> result = malloc.allocate<ffi.Uint8>(
    units.length + 1,
  );

  final Uint8List nativeBytes = result.asTypedList(units.length + 1);
  nativeBytes.setAll(0, units);
  nativeBytes[units.length] = 0;
  ffi.Pointer<ffi.Uint8> data = result.cast();
  int dataLength = units.length;

  return [data, dataLength];
}

typedef dartPostCObject = Pointer Function(
    Pointer<NativeFunction<Int8 Function(Int64, Pointer<Dart_CObject>)>>);
