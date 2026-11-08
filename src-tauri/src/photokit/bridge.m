// PhotoKit bridge. Every export blocks its caller, so call these only from worker
// threads. Results are malloc'd UTF-8 JSON, released with bk_free. NSExceptions are
// caught here because they must never unwind into Rust.
#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>
#import <Photos/Photos.h>
#include <stdlib.h>
#include <string.h>

// PHPhotosError values. NSUserCancelledError shares the cancelled value.
static const NSInteger kCancelled = 3072;
static const NSInteger kAccessRestricted = 3310;
static const NSInteger kAccessDenied = 3311;

static char *copy_json(id object) {
    if (![NSJSONSerialization isValidJSONObject:object]) return NULL;
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:NULL];
    if (!data) return NULL;
    char *out = malloc(data.length + 1);
    if (!out) return NULL;
    memcpy(out, data.bytes, data.length);
    out[data.length] = 0;
    return out;
}

static char *failure(NSString *code) { return copy_json(@{@"error" : code}); }

void bk_free(char *value) { free(value); }

static PHAuthorizationStatus current_status(void) {
    return [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
}

static NSString *status_name(PHAuthorizationStatus status) {
    switch (status) {
        case PHAuthorizationStatusNotDetermined: return @"notDetermined";
        case PHAuthorizationStatusRestricted: return @"restricted";
        case PHAuthorizationStatusLimited: return @"limited";
        case PHAuthorizationStatusAuthorized: return @"authorized";
        default: return @"denied";
    }
}

static BOOL has_access(void) {
    PHAuthorizationStatus status = current_status();
    return status == PHAuthorizationStatusAuthorized || status == PHAuthorizationStatusLimited;
}

static BOOL is_eligible(PHAsset *asset) {
    // A burst is one row standing in for many photos, so deleting it can't be an informed
    // choice about what the user saw.
    return asset.mediaType == PHAssetMediaTypeImage && !asset.hidden && !asset.representsBurst &&
           (asset.sourceType & PHAssetSourceTypeUserLibrary) != 0;
}

char *bk_status(void) {
    @autoreleasepool {
        @try {
            return copy_json(@{@"status" : status_name(current_status())});
        } @catch (NSException *e) {
            return failure(@"exception");
        }
    }
}

char *bk_request_access(void) {
    @autoreleasepool {
        @try {
            dispatch_semaphore_t done = dispatch_semaphore_create(0);
            __block PHAuthorizationStatus result = PHAuthorizationStatusDenied;
            [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelReadWrite
                                                       handler:^(PHAuthorizationStatus status) {
                                                           result = status;
                                                           dispatch_semaphore_signal(done);
                                                       }];
            dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
            return copy_json(@{@"status" : status_name(result)});
        } @catch (NSException *e) {
            return failure(@"exception");
        }
    }
}

// [[localIdentifier, creationMs, modificationMs], ...] oldest first, or {"error": code}.
char *bk_list(void) {
    @autoreleasepool {
        @try {
            if (!has_access()) return failure(@"denied");
            PHFetchOptions *options = [PHFetchOptions new];
            options.includeHiddenAssets = NO;
            options.includeAllBurstAssets = NO;
            options.includeAssetSourceTypes = PHAssetSourceTypeUserLibrary;
            options.sortDescriptors =
                @[ [NSSortDescriptor sortDescriptorWithKey:@"creationDate" ascending:YES] ];
            PHFetchResult<PHAsset *> *assets =
                [PHAsset fetchAssetsWithMediaType:PHAssetMediaTypeImage options:options];
            NSMutableArray *rows = [NSMutableArray arrayWithCapacity:assets.count];
            for (NSUInteger i = 0; i < assets.count; i++) {
                @autoreleasepool {
                    PHAsset *asset = [assets objectAtIndex:i];
                    if (!is_eligible(asset)) continue;
                    NSDate *created = asset.creationDate;
                    NSDate *modified = asset.modificationDate;
                    long long ms = created ? (long long)(created.timeIntervalSince1970 * 1000) : 0;
                    long long changed =
                        modified ? (long long)(modified.timeIntervalSince1970 * 1000) : 0;
                    [rows addObject:@[ asset.localIdentifier, @(ms), @(changed) ]];
                }
            }
            return copy_json(rows);
        } @catch (NSException *e) {
            return failure(@"exception");
        }
    }
}

// Photos itself asks the user to confirm, and moves the assets to Recently Deleted.
char *bk_delete(const char *ids_json) {
    @autoreleasepool {
        @try {
            if (!ids_json) return copy_json(@{@"outcome" : @"error", @"code" : @(-1)});
            NSData *input = [NSData dataWithBytes:ids_json length:strlen(ids_json)];
            NSArray *ids = [NSJSONSerialization JSONObjectWithData:input options:0 error:NULL];
            if (![ids isKindOfClass:[NSArray class]])
                return copy_json(@{@"outcome" : @"error", @"code" : @(-1)});
            for (id value in ids) {
                if (![value isKindOfClass:[NSString class]])
                    return copy_json(@{@"outcome" : @"error", @"code" : @(-1)});
            }
            // Without access a fetch comes back empty, which would look like missing photos.
            if (!has_access()) return copy_json(@{@"outcome" : @"denied"});

            PHFetchResult<PHAsset *> *fetched =
                [PHAsset fetchAssetsWithLocalIdentifiers:ids options:nil];
            NSMutableArray<PHAsset *> *assets = [NSMutableArray arrayWithCapacity:fetched.count];
            NSMutableSet<NSString *> *found = [NSMutableSet setWithCapacity:fetched.count];
            for (NSUInteger i = 0; i < fetched.count; i++) {
                PHAsset *asset = [fetched objectAtIndex:i];
                if (is_eligible(asset)) {
                    [assets addObject:asset];
                    [found addObject:asset.localIdentifier];
                }
            }
            NSMutableArray<NSString *> *missing = [NSMutableArray array];
            for (NSString *value in ids) {
                if (![found containsObject:value]) [missing addObject:value];
            }
            if (assets.count == 0) return copy_json(@{@"outcome" : @"deleted", @"missing" : missing});

            NSError *error = nil;
            BOOL ok = [[PHPhotoLibrary sharedPhotoLibrary]
                performChangesAndWait:^{
                    [PHAssetChangeRequest deleteAssets:assets];
                }
                                error:&error];
            if (ok) return copy_json(@{@"outcome" : @"deleted", @"missing" : missing});
            if (error.code == kCancelled) return copy_json(@{@"outcome" : @"cancelled"});
            if (error.code == kAccessDenied || error.code == kAccessRestricted)
                return copy_json(@{@"outcome" : @"denied"});
            return copy_json(@{@"outcome" : @"error", @"code" : @(error.code)});
        } @catch (NSException *e) {
            return copy_json(@{@"outcome" : @"error", @"code" : @(-2)});
        }
    }
}

// Returns nil on success, otherwise an error code. Synchronous requests can't be cancelled
// because their id arrives only after they finish, so requests that may touch the network
// run asynchronously and this thread waits with a deadline instead.
static NSString *fetch_image(PHAsset *asset, CGFloat edge, BOOL network, double timeout,
                             NSImage **image) {
    PHImageRequestOptions *options = [PHImageRequestOptions new];
    options.networkAccessAllowed = network;
    options.synchronous = !network;
    options.deliveryMode = network ? PHImageRequestOptionsDeliveryModeHighQualityFormat
                                   : PHImageRequestOptionsDeliveryModeOpportunistic;
    options.resizeMode =
        network ? PHImageRequestOptionsResizeModeExact : PHImageRequestOptionsResizeModeFast;

    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block NSImage *best = nil;
    __block NSString *problem = nil;
    PHImageManager *manager = [PHImageManager defaultManager];
    PHImageRequestID request = [manager
        requestImageForAsset:asset
                  targetSize:CGSizeMake(edge, edge)
                 contentMode:PHImageContentModeAspectFit
                     options:options
               resultHandler:^(NSImage *result, NSDictionary *info) {
                   BOOL degraded = [info[PHImageResultIsDegradedKey] boolValue];
                   if ([info[PHImageCancelledKey] boolValue]) {
                       problem = @"cancelled";
                   } else if (info[PHImageErrorKey]) {
                       NSError *error = info[PHImageErrorKey];
                       BOOL denied =
                           error.code == kAccessDenied || error.code == kAccessRestricted;
                       problem = denied ? @"denied" : @"icloud";
                   } else if (!result) {
                       problem = [info[PHImageResultIsInCloudKey] boolValue] ? @"icloud" : @"empty";
                   } else if (degraded && network) {
                       return;  // The final image follows.
                   } else {
                       best = result;
                   }
                   dispatch_semaphore_signal(done);
               }];
    dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, (int64_t)(timeout * NSEC_PER_SEC));
    if (dispatch_semaphore_wait(done, deadline) != 0) {
        [manager cancelImageRequest:request];
        return @"timeout";
    }
    if (problem) return problem;
    if (!best) return @"empty";
    *image = best;
    return nil;
}

static NSString *write_jpeg(NSImage *image, NSString *path) {
    CGImageRef cg = [image CGImageForProposedRect:NULL context:nil hints:nil];
    if (!cg) return @"empty";
    NSMutableData *data = [NSMutableData data];
    CGImageDestinationRef destination = CGImageDestinationCreateWithData(
        (__bridge CFMutableDataRef)data, CFSTR("public.jpeg"), 1, NULL);
    if (!destination) return @"write";
    NSDictionary *properties =
        @{(__bridge NSString *)kCGImageDestinationLossyCompressionQuality : @0.8};
    CGImageDestinationAddImage(destination, cg, (__bridge CFDictionaryRef)properties);
    BOOL finished = CGImageDestinationFinalize(destination);
    CFRelease(destination);
    if (!finished || data.length == 0) return @"write";
    // Non-atomic on purpose: keeps the private permissions of the file Rust created.
    return [data writeToFile:path options:0 error:NULL] ? nil : @"write";
}

// {"ok": true} or {"error": code}
char *bk_render(const char *c_id, int edge, int allow_network, const char *c_out, double timeout) {
    @autoreleasepool {
        @try {
            if (!c_id || !c_out || edge <= 0 || timeout <= 0) return failure(@"invalid");
            NSString *identifier = [NSString stringWithUTF8String:c_id];
            NSString *out = [NSString stringWithUTF8String:c_out];
            if (!identifier || !out) return failure(@"invalid");
            if (!has_access()) return failure(@"denied");
            PHAsset *asset =
                [PHAsset fetchAssetsWithLocalIdentifiers:@[ identifier ] options:nil].firstObject;
            if (!asset || !is_eligible(asset)) return failure(@"missing");

            BOOL network = allow_network != 0;
            NSImage *image = nil;
            NSString *problem = fetch_image(asset, edge, network, timeout, &image);
            // Without the network a smaller local rendition may still exist.
            if ([problem isEqualToString:@"icloud"] && !network) {
                problem = fetch_image(asset, edge / 2, NO, timeout, &image);
            }
            if (problem) return failure(problem);
            problem = write_jpeg(image, out);
            return problem ? failure(problem) : copy_json(@{@"ok" : @YES});
        } @catch (NSException *e) {
            return failure(@"exception");
        }
    }
}
